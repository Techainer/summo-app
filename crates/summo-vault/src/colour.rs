//! Colours a note can carry, and the rules that stop one from becoming a stylesheet.
//!
//! A colour is a **name from a fixed palette**, not a free hex string:
//!
//! ```markdown
//! ---
//! color: teal
//! ---
//! ```
//!
//! The field is spelled `color` because that is what CSS calls it and what anyone typing into
//! frontmatter will reach for; the module is spelled the way the rest of this codebase writes prose.
//!
//! ## Why names rather than hex
//!
//! **A hex chosen in one scheme is wrong in the other.** The app is dark by default and light when
//! the system asks. `#0f7350` is a readable green on white and nearly invisible on `#0b0c0e`. A
//! name resolves to a different value per scheme, so a note tagged `green` stays legible in both —
//! and the theme, which is where every other colour decision already lives, stays the one place
//! that decides what green looks like.
//!
//! **It removes an injection surface rather than guarding one.** The colour comes out of a file the
//! user edits by hand, and it ends up in a `style` attribute. A free-form string there is a place
//! to write `red; background: url(https://…)`. With a fixed palette the renderer only ever emits
//! `var(--swatch-<name>)` for a name it already knew, so there is nothing to escape and nothing to
//! get wrong later — the safety is structural rather than a validator somebody has to remember.
//!
//! ## And yet hex still works
//!
//! Somebody *will* type `color: "#0f7350"`, because that is what a colour looks like everywhere
//! else. Rejecting it would make the file format hostile to the tool the vault is meant to be
//! edited with. So a hex value is **snapped to the nearest palette name** — `#0f7350` becomes
//! `green` — and the user gets a colour rather than an error. Nothing is written back to their
//! file, so the hex they typed is still the hex in the file the next time they open it.

use serde::Serialize;

/// The palette, in the order a picker shows it.
///
/// Eight is enough to file a vault by and few enough to tell apart at a glance; a colour a user
/// cannot distinguish from its neighbour is a colour that does not organise anything.
///
/// The hex here is *not* what gets rendered — the theme owns that, once per scheme. It is the
/// reference point a hand-typed hex is snapped to, and it is a mid-tone between the two schemes'
/// values so the snapping does not favour one over the other.
pub const PALETTE: [Swatch; 8] = [
    Swatch::new("red", [0xd9, 0x45, 0x48]),
    Swatch::new("amber", [0xd4, 0x93, 0x3d]),
    Swatch::new("green", [0x2e, 0xa8, 0x6d]),
    Swatch::new("teal", [0x2b, 0x9c, 0xa8]),
    Swatch::new("blue", [0x4c, 0x83, 0xd6]),
    Swatch::new("purple", [0x8b, 0x6c, 0xd9]),
    Swatch::new("pink", [0xd0, 0x5c, 0x9e]),
    Swatch::new("grey", [0x83, 0x8b, 0x99]),
];

/// One entry in the palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Swatch {
    pub name: &'static str,
    /// Reference RGB, for snapping a hand-typed hex. Not for rendering.
    #[serde(skip)]
    pub rgb: [u8; 3],
}

impl Swatch {
    const fn new(name: &'static str, rgb: [u8; 3]) -> Self {
        Self { name, rgb }
    }
}

/// The palette name a raw frontmatter value means, or `None` if it means nothing.
///
/// `None` is deliberately not an error. A file with `color: chartreuse` in it is a file with one
/// field we do not understand, not a broken document, and a vault that refused to list it would be
/// worse than one that lists it without a dot.
#[must_use]
pub fn normalise(raw: &str) -> Option<&'static str> {
    let raw = raw.trim().trim_start_matches('#').trim();
    if raw.is_empty() {
        return None;
    }
    // A name first: it is the spelling we write and the common case.
    let lower = raw.to_ascii_lowercase();
    if let Some(swatch) = PALETTE.iter().find(|s| s.name == lower) {
        return Some(swatch.name);
    }
    // British and American spellings of the same colour, so neither is a wrong answer.
    if lower == "gray" {
        return Some("grey");
    }
    hex(&lower).map(nearest)
}

/// The palette name to store for a value the app itself is setting.
///
/// Stricter than [`normalise`] on purpose: the app has a picker, so anything arriving here that is
/// not a palette name is a bug in a caller rather than a person's typing, and saying so is more
/// useful than quietly writing something else into their file.
pub fn parse(raw: &str) -> Result<&'static str, summo_core::Error> {
    normalise(raw).ok_or_else(|| {
        summo_core::Error::Vault(format!(
            "{raw:?} is not a colour; use one of: {}",
            PALETTE
                .iter()
                .map(|s| s.name)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })
}

/// `rrggbb` or `rgb`, already lowercased and stripped of its `#`.
fn hex(text: &str) -> Option<[u8; 3]> {
    let expanded: String = match text.len() {
        // `#0f0` is three doubled nibbles, the same shorthand CSS uses.
        3 => text.chars().flat_map(|c| [c, c]).collect(),
        6 => text.to_string(),
        _ => return None,
    };
    let byte = |i: usize| u8::from_str_radix(expanded.get(i..i + 2)?, 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// The palette entry closest to an arbitrary colour.
///
/// Weighted Euclidean distance in sRGB — the cheap approximation, weighted 2:4:3 because the eye
/// gives green the most of its resolution and blue the least. It is not a colour-science answer and
/// does not need to be: the job is picking one of eight well-separated hues, and the cases where a
/// better metric would disagree are the ones where a person would not care which they got.
fn nearest(rgb: [u8; 3]) -> &'static str {
    const WEIGHTS: [i32; 3] = [2, 4, 3];
    PALETTE
        .iter()
        .min_by_key(|swatch| {
            (0..3)
                .map(|i| {
                    let d = i32::from(rgb[i]) - i32::from(swatch.rgb[i]);
                    WEIGHTS[i] * d * d
                })
                .sum::<i32>()
        })
        // The palette is a non-empty constant, so this cannot fail; `grey` is the honest fallback.
        .map_or("grey", |s| s.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_palette_name_is_itself() {
        assert_eq!(normalise("teal"), Some("teal"));
        assert_eq!(normalise("  TEAL  "), Some("teal"));
    }

    /// Neither spelling of this one is wrong, and a user should not have to guess ours.
    #[test]
    fn gray_and_grey_are_the_same_colour() {
        assert_eq!(normalise("gray"), Some("grey"));
        assert_eq!(normalise("grey"), Some("grey"));
    }

    /// The ADR's own example. Somebody typing a hex into Obsidian gets a colour, not an error.
    #[test]
    fn a_hand_typed_hex_snaps_to_the_nearest_name() {
        assert_eq!(normalise("#0f7350"), Some("green"));
        assert_eq!(normalise("#c62f32"), Some("red"));
        assert_eq!(normalise("#2f5fc0"), Some("blue"));
        assert_eq!(normalise("#888888"), Some("grey"));
    }

    #[test]
    fn css_three_digit_shorthand_expands_the_way_css_does() {
        assert_eq!(normalise("#0f0"), normalise("#00ff00"));
        assert_eq!(normalise("#0f0"), Some("green"));
    }

    /// The reason the palette exists: whatever arrives, what leaves is a name we chose, so the
    /// renderer only ever emits `var(--swatch-<known>)`.
    #[test]
    fn nothing_that_could_be_css_survives() {
        for attack in [
            "red; background: url(https://evil.example/pixel.png)",
            "}, body { display: none } .x {",
            "var(--color-bg)",
            "expression(alert(1))",
            "#0f7350; --color-bg: red",
        ] {
            assert_eq!(normalise(attack), None, "{attack:?} must not survive");
        }
    }

    #[test]
    fn an_unknown_word_is_no_colour_rather_than_an_error() {
        assert_eq!(normalise("chartreuse"), None);
        assert_eq!(normalise(""), None);
        assert_eq!(normalise("   "), None);
        assert_eq!(normalise("#12345"), None, "five digits is not a hex colour");
    }

    /// The app has a picker, so a value that is not a colour is a bug in a caller and worth saying.
    #[test]
    fn setting_a_colour_the_app_does_not_know_is_an_error_with_the_options_in_it() {
        let err = parse("chartreuse").unwrap_err().to_string();
        assert!(err.contains("chartreuse"), "{err}");
        assert!(err.contains("teal"), "the message must list what is allowed: {err}");
    }

    #[test]
    fn every_palette_name_round_trips_through_parse() {
        for swatch in PALETTE {
            assert_eq!(parse(swatch.name).unwrap(), swatch.name);
        }
    }

    /// Two names rendering as the same dot would be two ways to file a note that look identical.
    #[test]
    fn the_palette_has_no_duplicates() {
        for (i, a) in PALETTE.iter().enumerate() {
            for b in &PALETTE[i + 1..] {
                assert_ne!(a.name, b.name);
                assert_ne!(a.rgb, b.rgb);
            }
        }
    }

    /// Snapping must be stable: each reference colour has to choose itself, or the palette is not
    /// separated enough for the metric being used.
    #[test]
    fn every_swatch_is_its_own_nearest() {
        for swatch in PALETTE {
            assert_eq!(nearest(swatch.rgb), swatch.name);
        }
    }
}
