//! The on-disk format for stored embeddings.
//!
//! ADR 0003 measured the alternatives and chose this: raw `f32` records behind a fixed header, one
//! file per meeting, no database. JSON cost 2.9× the disk and 1.5 s of parsing at 200,000 vectors
//! for a payload with no human reader — the *encoding* was the mistake, not the decision to use
//! files.
//!
//! ```text
//! "SUMOVEC\x01"  magic and version
//! u32            dims
//! u32            sample count
//! str            embedding model id      ─┐ the coordinate system these vectors belong to;
//! str            model revision           │ comparing across it is meaningless, so it is
//! str            meeting id              ─┘ recorded rather than assumed
//! per sample:
//!   u64          seq
//!   f64          t0
//!   f64          duration
//!   u8           flags — bit 0 is "a human confirmed this"
//!   str          label
//!   str          person id, empty when unmatched
//!   f32 × dims   the embedding
//! ```
//!
//! Little-endian throughout, and `str` is a `u16` byte length followed by UTF-8.
//!
//! Two properties are worth stating because they are what a format like this gets wrong. **A short
//! file is an error, not a truncated read**: a vector missing half its dimensions would otherwise
//! compare as a valid vector pointing somewhere arbitrary. And **the dimension is in the header**,
//! so a file written by a different model is refused rather than reinterpreted.

use std::io::{Read, Write};

use summo_core::{Error, Result};

/// Magic plus format version. Bumped only for a breaking layout change.
const MAGIC: [u8; 8] = *b"SUMOVEC\x01";

/// Sanity ceiling on a declared length, so a corrupt header cannot ask for a terabyte.
///
/// 4,096 dimensions is an order of magnitude above any speaker embedder; 8 million samples is
/// several lifetimes of meetings.
const MAX_DIMS: usize = 4_096;
const MAX_SAMPLES: usize = 8_000_000;

/// What a reader needs to know before it touches a vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub dims: usize,
    pub count: usize,
    pub model: String,
    pub revision: String,
    pub meeting: String,
}

/// Whether `bytes` begins with this format's magic.
///
/// Used to tell a binary log from the JSON one a previous release wrote, so an existing vault is
/// read rather than rejected.
#[must_use]
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.len() >= MAGIC.len() && bytes[..MAGIC.len()] == MAGIC
}

/// One record, decoupled from `VoiceSample` so this module stays a codec.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub seq: u64,
    pub t0: f64,
    pub duration: f64,
    pub confirmed: bool,
    pub label: String,
    pub person: Option<String>,
    pub embedding: Vec<f32>,
}

/// Encode a whole file.
pub fn write(header: &Header, records: &[Record]) -> Result<Vec<u8>> {
    // Pre-size to the exact payload: this is the hot path when a meeting ends.
    let mut out = Vec::with_capacity(64 + records.len() * (33 + header.dims * 4));
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&(header.dims as u32).to_le_bytes());
    out.extend_from_slice(&(records.len() as u32).to_le_bytes());
    put_str(&mut out, &header.model)?;
    put_str(&mut out, &header.revision)?;
    put_str(&mut out, &header.meeting)?;

    for record in records {
        if record.embedding.len() != header.dims {
            return Err(Error::Other(format!(
                "sample {} has {} dimensions, the file declares {}",
                record.seq,
                record.embedding.len(),
                header.dims
            )));
        }
        out.extend_from_slice(&record.seq.to_le_bytes());
        out.extend_from_slice(&record.t0.to_le_bytes());
        out.extend_from_slice(&record.duration.to_le_bytes());
        out.push(u8::from(record.confirmed));
        put_str(&mut out, &record.label)?;
        put_str(&mut out, record.person.as_deref().unwrap_or(""))?;
        for value in &record.embedding {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    Ok(out)
}

/// Decode a whole file.
pub fn read(bytes: &[u8]) -> Result<(Header, Vec<Record>)> {
    let mut cursor = Cursor::new(bytes);

    let magic = cursor.take(MAGIC.len())?;
    if magic != MAGIC {
        return Err(Error::Other("not a Summo vector file".into()));
    }

    let dims = cursor.u32()? as usize;
    let count = cursor.u32()? as usize;
    if dims == 0 || dims > MAX_DIMS {
        return Err(Error::Other(format!("implausible dimension {dims}")));
    }
    if count > MAX_SAMPLES {
        return Err(Error::Other(format!("implausible sample count {count}")));
    }

    let header = Header {
        dims,
        count,
        model: cursor.string()?,
        revision: cursor.string()?,
        meeting: cursor.string()?,
    };

    let mut records = Vec::with_capacity(count.min(4_096));
    for _ in 0..count {
        let seq = cursor.u64()?;
        let t0 = f64::from_bits(cursor.u64()?);
        let duration = f64::from_bits(cursor.u64()?);
        let confirmed = cursor.take(1)?[0] & 1 == 1;
        let label = cursor.string()?;
        let person = cursor.string()?;

        let raw = cursor.take(dims * 4)?;
        let embedding: Vec<f32> = raw
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();

        records.push(Record {
            seq,
            t0,
            duration,
            confirmed,
            label,
            person: (!person.is_empty()).then_some(person),
            embedding,
        });
    }
    Ok((header, records))
}

/// Read only the header, without decoding a single vector.
///
/// This is what makes a space check cheap: deciding whether a 150 MB history is comparable with the
/// running model should not cost 150 MB of reading.
pub fn read_header(reader: &mut impl Read) -> Result<Header> {
    // Enough for the fixed part plus three short strings.
    let mut buffer = [0u8; 1024];
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(Error::Other(format!("cannot read header: {e}"))),
        }
    }

    let mut cursor = Cursor::new(&buffer[..filled]);
    if cursor.take(MAGIC.len())? != MAGIC {
        return Err(Error::Other("not a Summo vector file".into()));
    }
    let dims = cursor.u32()? as usize;
    let count = cursor.u32()? as usize;
    Ok(Header {
        dims,
        count,
        model: cursor.string()?,
        revision: cursor.string()?,
        meeting: cursor.string()?,
    })
}

fn put_str(out: &mut Vec<u8>, value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    let len = u16::try_from(bytes.len())
        .map_err(|_| Error::Other(format!("string too long for this format: {} bytes", bytes.len())))?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

/// A bounds-checked reader over a byte slice.
///
/// Every accessor returns an error rather than panicking on a short buffer, because these files can
/// be truncated by a full disk or a crash mid-write, and a partial vector must be refused rather
/// than silently compared.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).ok_or_else(|| truncated(self.at, n))?;
        if end > self.bytes.len() {
            return Err(truncated(self.at, n));
        }
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn string(&mut self) -> Result<String> {
        let b = self.take(2)?;
        let len = u16::from_le_bytes([b[0], b[1]]) as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|_| Error::Other("a string in this file is not valid UTF-8".into()))
    }
}

fn truncated(at: usize, wanted: usize) -> Error {
    Error::Other(format!(
        "vector file is truncated: wanted {wanted} bytes at offset {at}"
    ))
}

/// Write `bytes` to `path` through a temporary file and a rename.
///
/// A half-written log read at the start of the next meeting would lose a meeting's vectors, which
/// is the difference between "Summo forgot one voice" and "Summo forgot who everyone is".
pub fn write_atomically(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    let temporary = path.with_extension("tmp");
    let mut file = std::fs::File::create(&temporary).map_err(|e| Error::io(&temporary, e))?;
    file.write_all(bytes).map_err(|e| Error::io(&temporary, e))?;
    file.sync_all().map_err(|e| Error::io(&temporary, e))?;
    std::fs::rename(&temporary, path).map_err(|e| Error::io(path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(dims: usize, count: usize) -> Header {
        Header {
            dims,
            count,
            model: "campplus-sv".into(),
            revision: "2026-01".into(),
            meeting: "01ABC".into(),
        }
    }

    fn record(seq: u64, dims: usize) -> Record {
        Record {
            seq,
            t0: 12.5,
            duration: 3.25,
            confirmed: seq.is_multiple_of(2),
            label: "S2".into(),
            person: (seq > 0).then(|| "ngoc".to_string()),
            embedding: (0..dims).map(|i| i as f32 / 10.0).collect(),
        }
    }

    #[test]
    fn a_file_round_trips_exactly() {
        let records: Vec<Record> = (0..5).map(|i| record(i, 192)).collect();
        let bytes = write(&header(192, records.len()), &records).expect("write");
        let (head, back) = read(&bytes).expect("read");

        assert_eq!(head.dims, 192);
        assert_eq!(head.count, 5);
        assert_eq!(head.model, "campplus-sv");
        assert_eq!(head.revision, "2026-01");
        assert_eq!(head.meeting, "01ABC");
        assert_eq!(back, records, "every field must survive the trip");
    }

    #[test]
    fn an_empty_log_is_a_valid_file() {
        let bytes = write(&header(192, 0), &[]).expect("write");
        let (head, back) = read(&bytes).expect("read");
        assert_eq!(head.count, 0);
        assert!(back.is_empty());
    }

    #[test]
    fn vietnamese_names_survive() {
        let mut r = record(1, 4);
        r.person = Some("nguyễn-thị-ngọc".into());
        r.label = "Người 2".into();
        let bytes = write(&header(4, 1), std::slice::from_ref(&r)).expect("write");
        let (_, back) = read(&bytes).expect("read");
        assert_eq!(back[0].person.as_deref(), Some("nguyễn-thị-ngọc"));
        assert_eq!(back[0].label, "Người 2");
    }

    #[test]
    fn an_unmatched_sample_reads_back_as_unmatched() {
        // Empty string on disk means "nobody", not a person whose id is "".
        let bytes = write(&header(4, 1), &[record(0, 4)]).expect("write");
        let (_, back) = read(&bytes).expect("read");
        assert_eq!(back[0].person, None);
    }

    #[test]
    fn the_confirmed_flag_survives() {
        let records = vec![record(0, 4), record(1, 4)];
        let bytes = write(&header(4, 2), &records).expect("write");
        let (_, back) = read(&bytes).expect("read");
        assert!(back[0].confirmed);
        assert!(!back[1].confirmed);
    }

    /// The property that matters most: a partial vector must not read as a whole one.
    #[test]
    fn a_truncated_file_is_an_error_not_a_short_vector() {
        let bytes = write(&header(192, 2), &[record(0, 192), record(1, 192)]).expect("write");
        for cut in [bytes.len() - 1, bytes.len() - 400, bytes.len() / 2, 20, 8] {
            let err = read(&bytes[..cut]).expect_err("truncation must be refused");
            assert!(
                err.to_string().contains("truncated") || err.to_string().contains("not a Summo"),
                "unexpected error at cut {cut}: {err}"
            );
        }
    }

    #[test]
    fn an_empty_file_is_refused() {
        assert!(read(&[]).is_err());
    }

    #[test]
    fn a_json_file_is_not_mistaken_for_this_format() {
        let json = br#"{"meeting":"01A","samples":[]}"#;
        assert!(!is_binary(json));
        assert!(read(json).is_err());
    }

    #[test]
    fn magic_identifies_our_own_files() {
        let bytes = write(&header(4, 0), &[]).expect("write");
        assert!(is_binary(&bytes));
    }

    #[test]
    fn a_corrupt_dimension_is_refused_rather_than_allocated() {
        let mut bytes = write(&header(4, 0), &[]).expect("write");
        bytes[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        let err = read(&bytes).expect_err("a huge dimension must be refused");
        assert!(err.to_string().contains("implausible"), "{err}");
    }

    #[test]
    fn a_corrupt_count_is_refused_rather_than_allocated() {
        let mut bytes = write(&header(4, 0), &[]).expect("write");
        bytes[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        let err = read(&bytes).expect_err("a huge count must be refused");
        assert!(err.to_string().contains("implausible"), "{err}");
    }

    #[test]
    fn a_sample_of_the_wrong_width_is_refused_at_write_time() {
        let mut r = record(0, 4);
        r.embedding.push(1.0);
        let err = write(&header(4, 1), &[r]).expect_err("a mismatched vector must not be written");
        assert!(err.to_string().contains("dimensions"), "{err}");
    }

    #[test]
    fn the_header_can_be_read_without_the_vectors() {
        let records: Vec<Record> = (0..1_000).map(|i| record(i, 192)).collect();
        let bytes = write(&header(192, records.len()), &records).expect("write");

        let mut reader = std::io::Cursor::new(&bytes);
        let head = read_header(&mut reader).expect("header");
        assert_eq!(head.dims, 192);
        assert_eq!(head.count, 1_000);
        assert_eq!(head.model, "campplus-sv");
    }

    /// A unit vector with the messy mantissas a real embedder produces.
    ///
    /// The values matter to this test: `0.1`, `0.2` serialise to three characters, while a real
    /// embedding component serialises to eleven. Testing with tidy numbers would measure a saving
    /// that does not exist in production.
    fn realistic(seed: u64, dims: usize) -> Vec<f32> {
        let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let mut out = Vec::with_capacity(dims);
        for _ in 0..dims {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            out.push((state >> 33) as f32 / f32::from(u16::MAX) - 1.0);
        }
        let norm = out.iter().map(|x| x * x).sum::<f32>().sqrt();
        out.iter().map(|x| x / norm).collect()
    }

    #[test]
    fn the_binary_form_is_much_smaller_than_json() {
        let records: Vec<Record> = (0..500)
            .map(|i| Record {
                embedding: realistic(i, 192),
                ..record(i, 192)
            })
            .collect();

        let binary = write(&header(192, records.len()), &records).expect("write");
        // The vectors are all but the whole payload, so comparing them is comparing the files.
        let vectors: Vec<&Vec<f32>> = records.iter().map(|r| &r.embedding).collect();
        let json = serde_json::to_vec(&vectors).expect("serialise");

        assert!(
            binary.len() * 2 < json.len(),
            "binary {} should be under half of json {}",
            binary.len(),
            json.len()
        );
    }

    #[test]
    fn writing_atomically_leaves_no_temporary_behind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("01A.vec");
        let bytes = write(&header(4, 1), &[record(0, 4)]).expect("encode");
        write_atomically(&path, &bytes).expect("write");

        assert_eq!(std::fs::read(&path).expect("read back"), bytes);
        assert!(!path.with_extension("tmp").exists());
    }
}
