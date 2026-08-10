//! File names for meetings.
//!
//! Vault files are meant to be opened in Obsidian, listed in a terminal and synced by whatever the
//! user already uses, so names have to survive case-insensitive filesystems, Windows' reserved
//! characters, and non-ASCII input.
//!
//! Vietnamese needs particular care: `Đánh giá Q3` must not become `nh-gi-q3`. Diacritics are
//! folded to their base letters — `đ` to `d`, `ề` to `e` — rather than dropped.

/// Vietnamese letters that decompose to a base ASCII letter.
///
/// Latin-1 accents fold naturally through NFD-style rules, but `đ`/`Đ` and the vowels carrying two
/// marks do not, so the mapping is explicit.
pub(crate) fn fold_char(c: char) -> Option<char> {
    let folded = match c {
        'à' | 'á' | 'ạ' | 'ả' | 'ã' | 'â' | 'ầ' | 'ấ' | 'ậ' | 'ẩ' | 'ẫ' | 'ă' | 'ằ' | 'ắ' | 'ặ'
        | 'ẳ' | 'ẵ' => 'a',
        'è' | 'é' | 'ẹ' | 'ẻ' | 'ẽ' | 'ê' | 'ề' | 'ế' | 'ệ' | 'ể' | 'ễ' => 'e',
        'ì' | 'í' | 'ị' | 'ỉ' | 'ĩ' => 'i',
        'ò' | 'ó' | 'ọ' | 'ỏ' | 'õ' | 'ô' | 'ồ' | 'ố' | 'ộ' | 'ổ' | 'ỗ' | 'ơ' | 'ờ' | 'ớ' | 'ợ'
        | 'ở' | 'ỡ' => 'o',
        'ù' | 'ú' | 'ụ' | 'ủ' | 'ũ' | 'ư' | 'ừ' | 'ứ' | 'ự' | 'ử' | 'ữ' => 'u',
        'ỳ' | 'ý' | 'ỵ' | 'ỷ' | 'ỹ' => 'y',
        'đ' => 'd',
        _ => return None,
    };
    Some(folded)
}

/// Names Windows refuses regardless of extension.
const RESERVED_NAMES: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Turn a title into a file-name-safe slug.
///
/// Always returns something usable: a title of pure punctuation becomes `untitled`.
#[must_use]
pub fn slugify(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut last_dash = true;

    for ch in title.chars() {
        let lowered: Vec<char> = ch.to_lowercase().collect();
        for c in lowered {
            let mapped = fold_char(c).unwrap_or(c);
            if mapped.is_ascii_alphanumeric() {
                out.push(mapped);
                last_dash = false;
            } else if !last_dash {
                // Everything else — punctuation, the characters Windows forbids, emoji — becomes a
                // separator rather than vanishing, so `1:1 with Ngoc` reads as `1-1-with-ngoc`.
                out.push('-');
                last_dash = true;
            }
        }
    }

    let trimmed = out.trim_matches('-');
    // Long names break on some filesystems and are unreadable in a file list either way.
    let mut slug: String = trimmed.chars().take(60).collect();
    slug = slug.trim_end_matches('-').to_string();

    if slug.is_empty() {
        return "untitled".into();
    }
    if RESERVED_NAMES.contains(&slug.as_str()) {
        return format!("{slug}-meeting");
    }
    slug
}

/// Build the file stem for a meeting: `YYYY-MM-DD-slug`.
///
/// The date leads so a directory listing is chronological without any tooling.
#[must_use]
pub fn meeting_stem(date: &str, title: &str) -> String {
    format!("{date}-{}", slugify(title))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_titles_become_lowercase_hyphenated() {
        assert_eq!(slugify("Weekly Sync"), "weekly-sync");
        assert_eq!(slugify("1:1 with Ngoc"), "1-1-with-ngoc");
    }

    #[test]
    fn vietnamese_diacritics_fold_rather_than_disappear() {
        assert_eq!(slugify("Đánh giá Q3"), "danh-gia-q3");
        assert_eq!(slugify("Họp nhóm sản phẩm"), "hop-nhom-san-pham");
        assert_eq!(slugify("Tuần này ưu tiên gì"), "tuan-nay-uu-tien-gi");
    }

    #[test]
    fn every_vietnamese_vowel_family_folds() {
        // A missed row here silently mangles a whole class of titles.
        for (input, expected) in [
            ("àáạảãâầấậẩẫăằắặẳẵ", "aaaaaaaaaaaaaaaaa"),
            ("èéẹẻẽêềếệểễ", "eeeeeeeeeee"),
            ("ìíịỉĩ", "iiiii"),
            ("òóọỏõôồốộổỗơờớợởỡ", "ooooooooooooooooo"),
            ("ùúụủũưừứựửữ", "uuuuuuuuuuu"),
            ("ỳýỵỷỹ", "yyyyy"),
            ("đĐ", "dd"),
        ] {
            assert_eq!(slugify(input), expected, "folding failed for {input}");
        }
    }

    #[test]
    fn reserved_characters_never_reach_the_filesystem() {
        // The set Windows rejects outright, plus the separators that would change the path.
        const RESERVED: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
        let slug = slugify(r#"Q3: plan/review <draft> "final"?"#);
        assert!(
            !slug.chars().any(|c| RESERVED.contains(&c)),
            "slug still contains a reserved character: {slug}"
        );
        assert_eq!(slug, "q3-plan-review-draft-final");
        assert!(
            !slug.contains(".."),
            "slug must not enable traversal: {slug}"
        );
    }

    #[test]
    fn windows_reserved_names_are_avoided() {
        assert_eq!(slugify("CON"), "con-meeting");
        assert_eq!(slugify("nul"), "nul-meeting");
    }

    #[test]
    fn a_title_of_punctuation_still_yields_a_name() {
        assert_eq!(slugify("!!! ???"), "untitled");
        assert_eq!(slugify(""), "untitled");
        assert_eq!(slugify("   "), "untitled");
    }

    #[test]
    fn very_long_titles_are_truncated_cleanly() {
        let slug = slugify(&"cuộc họp rất dài ".repeat(20));
        assert!(slug.len() <= 60, "got {} chars", slug.len());
        assert!(
            !slug.ends_with('-'),
            "truncation left a trailing dash: {slug}"
        );
    }

    #[test]
    fn separators_do_not_repeat_or_dangle() {
        assert_eq!(slugify("  spaced   out  "), "spaced-out");
        assert_eq!(slugify("--dashes--"), "dashes");
    }

    #[test]
    fn meeting_stems_sort_chronologically() {
        let mut names = vec![
            meeting_stem("2026-08-09", "Weekly Sync"),
            meeting_stem("2026-01-02", "Kickoff"),
            meeting_stem("2026-08-01", "Đánh giá"),
        ];
        names.sort();
        assert_eq!(
            names,
            vec![
                "2026-01-02-kickoff",
                "2026-08-01-danh-gia",
                "2026-08-09-weekly-sync"
            ]
        );
    }
}
