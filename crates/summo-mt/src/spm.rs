//! Reading a SentencePiece model file.
//!
//! `sentencepiece.bpe.model` is a protobuf, and the obvious way to read it is to depend on
//! `sentencepiece` — which vendors Google's C++ library and its own copy of protobuf. This crate
//! exists so translation can run without a C++ toolchain, so paying one to read a vocabulary would
//! defeat the point.
//!
//! What is actually needed from the file is two things, and the wire format for both is stable and
//! trivial:
//!
//! * **The pieces** — 128 000 `(string, score)` pairs that the Unigram segmenter scores against.
//! * **The precompiled charsmap** — SentencePiece's own normalisation table (`nmt_nfkc`). Not
//!   optional: skipping it means a Vietnamese word written with combining marks tokenises
//!   differently from the same word in composed form, and the translation quietly degrades for text
//!   nobody can see is different.
//!
//! Everything else in the file — the trainer spec, the self-test data — is skipped.
//!
//! ```text
//! ModelProto {
//!   repeated SentencePiece pieces = 1 { string piece = 1; float score = 2; int32 type = 3; }
//!   TrainerSpec trainer_spec = 2;   // skipped
//!   NormalizerSpec normalizer = 3 { string name = 1; bytes precompiled_charsmap = 2; }
//! }
//! ```

use summo_core::{Error, Result};

/// A parsed SentencePiece model.
pub struct SpmModel {
    /// Piece and its score, in vocabulary order.
    ///
    /// For a BPE model — which is what SMALL100 ships — the score is the negated rank, so the
    /// list is already in merge order and the numbers are `0, -1, -2, …` rather than log
    /// probabilities. That is how [`is_bpe`] tells the two apart.
    pub pieces: Vec<(String, f64)>,
    /// SentencePiece's normalisation table, for `spm_precompiled`.
    pub charsmap: Vec<u8>,
    /// Index of `<unk>`, which the Unigram segmenter needs for a character it has never seen.
    pub unk_id: usize,
}

/// One protobuf field: its number, and its payload.
enum Field<'a> {
    Varint(u64),
    Fixed32([u8; 4]),
    Fixed64,
    Bytes(&'a [u8]),
}

/// Walk a protobuf message, handing each field to `visit`.
///
/// Unknown fields are skipped rather than rejected: the file is written by a different program's
/// current version, and a new field appearing in it is not a reason to refuse to translate.
fn walk(mut body: &[u8], mut visit: impl FnMut(u32, Field<'_>) -> Result<()>) -> Result<()> {
    while !body.is_empty() {
        let (key, rest) = varint(body)?;
        body = rest;
        let number = u32::try_from(key >> 3).map_err(|_| bad("field number out of range"))?;
        let (field, rest) = match key & 7 {
            0 => {
                let (value, rest) = varint(body)?;
                (Field::Varint(value), rest)
            }
            1 => {
                let rest = body.get(8..).ok_or_else(|| bad("truncated fixed64"))?;
                (Field::Fixed64, rest)
            }
            2 => {
                let (len, rest) = varint(body)?;
                let len = usize::try_from(len).map_err(|_| bad("length out of range"))?;
                let value = rest.get(..len).ok_or_else(|| bad("truncated bytes"))?;
                (Field::Bytes(value), &rest[len..])
            }
            5 => {
                let value: [u8; 4] = body
                    .get(..4)
                    .and_then(|b| b.try_into().ok())
                    .ok_or_else(|| bad("truncated fixed32"))?;
                (Field::Fixed32(value), &body[4..])
            }
            other => return Err(bad(&format!("unsupported wire type {other}"))),
        };
        body = rest;
        visit(number, field)?;
    }
    Ok(())
}

fn varint(body: &[u8]) -> Result<(u64, &[u8])> {
    let mut value = 0_u64;
    let mut shift = 0_u32;
    for (i, byte) in body.iter().enumerate() {
        if shift >= 64 {
            return Err(bad("varint too long"));
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, &body[i + 1..]));
        }
        shift += 7;
    }
    Err(bad("truncated varint"))
}

fn bad(reason: &str) -> Error {
    Error::Other(format!("not a sentencepiece model: {reason}"))
}

/// Parse `sentencepiece.bpe.model`.
pub fn parse(body: &[u8]) -> Result<SpmModel> {
    let mut pieces = Vec::new();
    let mut charsmap = Vec::new();
    let mut unk_id = None;

    walk(body, |number, field| {
        match (number, field) {
            (1, Field::Bytes(piece)) => {
                let (text, score, kind) = parse_piece(piece)?;
                // Type 2 is `UNKNOWN`. There is exactly one, and the segmenter needs its index.
                if kind == 2 && unk_id.is_none() {
                    unk_id = Some(pieces.len());
                }
                pieces.push((text, f64::from(score)));
            }
            (3, Field::Bytes(spec)) => {
                walk(spec, |number, field| {
                    if let (2, Field::Bytes(map)) = (number, field) {
                        charsmap = map.to_vec();
                    }
                    Ok(())
                })?;
            }
            _ => {}
        }
        Ok(())
    })?;

    if pieces.is_empty() {
        return Err(bad("no pieces"));
    }
    Ok(SpmModel {
        unk_id: unk_id.ok_or_else(|| bad("no <unk> piece"))?,
        pieces,
        charsmap,
    })
}

/// Whether this is a BPE model rather than a Unigram one.
///
/// The distinction decides the entire segmentation algorithm, and the file does not say which it is
/// anywhere this parser reads — but it does not have to. A Unigram model stores log probabilities,
/// which are irregular negative reals; a BPE model stores the negated merge rank, so the scores
/// come out as `0, -1, -2, -3, …` in order. Checking a handful of them is enough.
///
/// Getting this wrong is not a crash. Running Viterbi over merge ranks produces *plausible* pieces —
/// `▁C`, `hi`, `ều` where SentencePiece gives `▁Chi`, `ều` — and the model then translates the
/// wrong tokens into a fluent sentence about something else. That is how this was found: the
/// translation was grammatical English and had nothing to do with the input.
#[must_use]
pub fn is_bpe(pieces: &[(String, f64)]) -> bool {
    // Skip the control pieces at the front, which score 0 in both kinds.
    pieces
        .iter()
        .skip(3)
        .take(16)
        .enumerate()
        .all(|(i, (_, score))| (score + i as f64).abs() < 1e-6)
}

/// Reconstruct BPE merges from a ranked vocabulary.
///
/// SentencePiece stores a BPE model as pieces in merge order and does not store the merge list, so
/// it has to be derived: a piece exists because two lower-ranked pieces were merged, and the split
/// that produced it is the one where both halves are in the vocabulary. Ordering matters as much as
/// membership — merges are applied in rank order, and the same vocabulary with the merges shuffled
/// segments differently.
///
/// This is the algorithm `transformers` uses to convert a SentencePiece BPE model to a fast one,
/// reimplemented rather than depended on. It is quadratic in piece length and linear in vocabulary,
/// which is about 128 000 × 8 lookups — tens of milliseconds, once, at load.
#[must_use]
pub fn merges(pieces: &[(String, f64)]) -> Vec<(String, String)> {
    let rank: std::collections::HashMap<&str, usize> = pieces
        .iter()
        .enumerate()
        .map(|(i, (piece, _))| (piece.as_str(), i))
        .collect();

    let mut out: Vec<(usize, usize, usize, &str, &str)> = Vec::with_capacity(pieces.len());
    for (index, (piece, _)) in pieces.iter().enumerate() {
        // Split at character boundaries. Splitting bytes would invent merges between halves of a
        // Vietnamese diacritic.
        for (at, _) in piece.char_indices().skip(1) {
            let (left, right) = piece.split_at(at);
            if let (Some(&l), Some(&r)) = (rank.get(left), rank.get(right)) {
                out.push((index, l, r, left, right));
            }
        }
    }
    // By the merged piece first, then by the halves: two ways to build the same piece must be
    // applied in a fixed order, or segmentation depends on hash iteration order.
    out.sort_unstable_by_key(|(index, l, r, _, _)| (*index, *l, *r));
    out.into_iter()
        .map(|(_, _, _, left, right)| (left.to_string(), right.to_string()))
        .collect()
}

/// One `SentencePiece { piece = 1, score = 2, type = 3 }`.
fn parse_piece(body: &[u8]) -> Result<(String, f32, u64)> {
    let mut text = None;
    let mut score = 0.0_f32;
    // `NORMAL`, which is what a piece is when the field is absent — protobuf omits defaults.
    let mut kind = 1_u64;

    walk(body, |number, field| {
        match (number, field) {
            (1, Field::Bytes(value)) => {
                text = Some(
                    std::str::from_utf8(value)
                        .map_err(|_| bad("a piece is not valid UTF-8"))?
                        .to_string(),
                );
            }
            (2, Field::Fixed32(value)) => score = f32::from_le_bytes(value),
            (3, Field::Varint(value)) => kind = value,
            _ => {}
        }
        Ok(())
    })?;

    Ok((text.ok_or_else(|| bad("a piece has no text"))?, score, kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a message by hand, so the parser is tested against bytes rather than against a file
    /// that happens to be on this machine.
    fn piece(text: &str, score: f32, kind: u64) -> Vec<u8> {
        let mut out = vec![0x0a, text.len() as u8];
        out.extend_from_slice(text.as_bytes());
        out.push(0x15);
        out.extend_from_slice(&score.to_le_bytes());
        if kind != 1 {
            out.push(0x18);
            out.push(kind as u8);
        }
        out
    }

    fn message(pieces: &[Vec<u8>], charsmap: Option<&[u8]>) -> Vec<u8> {
        let mut out = Vec::new();
        for p in pieces {
            out.push(0x0a);
            out.push(p.len() as u8);
            out.extend_from_slice(p);
        }
        if let Some(map) = charsmap {
            let mut spec = vec![0x12, map.len() as u8];
            spec.extend_from_slice(map);
            out.push(0x1a);
            out.push(spec.len() as u8);
            out.extend_from_slice(&spec);
        }
        out
    }

    #[test]
    fn pieces_come_back_in_order_with_their_scores() {
        let body = message(
            &[
                piece("<unk>", 0.0, 2),
                piece("▁xin", -3.5, 1),
                piece("chào", -4.25, 1),
            ],
            None,
        );
        let model = parse(&body).unwrap();
        assert_eq!(model.pieces.len(), 3);
        assert_eq!(model.pieces[1].0, "▁xin");
        assert!((model.pieces[1].1 - -3.5).abs() < 1e-6);
        assert_eq!(
            model.unk_id, 0,
            "the UNKNOWN piece is the one the segmenter needs"
        );
    }

    #[test]
    fn the_normalisation_table_is_read() {
        let body = message(&[piece("<unk>", 0.0, 2)], Some(&[1, 2, 3, 4]));
        assert_eq!(parse(&body).unwrap().charsmap, vec![1, 2, 3, 4]);
    }

    /// The file is written by a different program's current version. A field this parser has never
    /// heard of is not a reason to refuse to translate.
    #[test]
    fn unknown_fields_are_skipped() {
        let mut body = message(&[piece("<unk>", 0.0, 2)], None);
        body.extend_from_slice(&[0x40, 0x2a]); // field 8, varint
        body.extend_from_slice(&[0x52, 0x02, 0xff, 0xee]); // field 10, bytes
        assert_eq!(parse(&body).unwrap().pieces.len(), 1);
    }

    /// A truncated download is a real failure mode and must not panic — this runs over bytes from
    /// the network, and a slice index would take the daemon down.
    #[test]
    fn a_truncated_file_is_an_error_not_a_panic() {
        let body = message(&[piece("<unk>", 0.0, 2), piece("▁xin", -1.0, 1)], None);
        for cut in 1..body.len() {
            let _ = parse(&body[..cut]);
        }
    }

    #[test]
    fn a_file_with_no_pieces_is_refused() {
        assert!(parse(&[]).is_err());
    }

    /// Without an `<unk>` the Unigram segmenter has nothing to emit for a character it has never
    /// seen, which is a crash the first time somebody types an emoji.
    #[test]
    fn a_file_with_no_unknown_piece_is_refused() {
        let body = message(&[piece("▁xin", -1.0, 1)], None);
        assert!(parse(&body).is_err());
    }
}
