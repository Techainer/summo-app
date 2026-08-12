//! Language codes and the names a model wants to be told.
//!
//! Two different things are called "the language" in this codebase and they are not interchangeable.
//! A **tag** — `vi`, `ja`, `zh` — is what the settings file, the ASR model and the translation file
//! on disk all use. A **name** — "Vietnamese", "Japanese", "Chinese (Simplified)" — is what goes
//! into a prompt, because that is what models were trained on. Passing `vi` where a name belonged
//! produced translations into whatever the model guessed, and it guessed differently per request.
//!
//! The list is the 46 languages of MiLMMT, which is the translation model this exists for. It is
//! deliberately not "every ISO 639-1 code": a name here is a claim that a translation into it will
//! be good, and 180 codes' worth of that claim would be false.
//!
//! Everything not on the list still goes through. [`name`] hands back whatever it was given, so
//! asking for a language the table does not know is a request the model may well satisfy — it just
//! is not one Summo is vouching for.

/// Tag and the name to use in a prompt.
///
/// Sorted by tag so the table is searchable by eye; `zh` is Simplified and `zh-Hant` Traditional,
/// which matches the tags the interface uses.
const NAMES: &[(&str, &str)] = &[
    ("ar", "Arabic"),
    ("az", "Azerbaijani"),
    ("bg", "Bulgarian"),
    ("bn", "Bengali"),
    ("ca", "Catalan"),
    ("cs", "Czech"),
    ("da", "Danish"),
    ("de", "German"),
    ("el", "Greek"),
    ("en", "English"),
    ("es", "Spanish"),
    ("fa", "Persian"),
    ("fi", "Finnish"),
    ("fr", "French"),
    ("he", "Hebrew"),
    ("hi", "Hindi"),
    ("hr", "Croatian"),
    ("hu", "Hungarian"),
    ("id", "Indonesian"),
    ("it", "Italian"),
    ("ja", "Japanese"),
    ("kk", "Kazakh"),
    ("km", "Khmer"),
    ("ko", "Korean"),
    ("lo", "Lao"),
    ("ms", "Malay"),
    ("my", "Burmese"),
    ("nb", "Norwegian"),
    ("nl", "Dutch"),
    ("no", "Norwegian"),
    ("pl", "Polish"),
    ("pt", "Portuguese"),
    ("ro", "Romanian"),
    ("ru", "Russian"),
    ("sk", "Slovak"),
    ("sl", "Slovenian"),
    ("sv", "Swedish"),
    ("ta", "Tamil"),
    ("th", "Thai"),
    ("tl", "Tagalog"),
    ("tr", "Turkish"),
    ("ur", "Urdu"),
    ("uz", "Uzbek"),
    ("vi", "Vietnamese"),
    ("yue", "Cantonese"),
    ("zh", "Chinese (Simplified)"),
    ("zh-hans", "Chinese (Simplified)"),
    ("zh-hant", "Chinese (Traditional)"),
];

/// The name to put in a prompt for `language`.
///
/// Accepts a tag (`vi`), a regional tag (`vi-VN`), or a name already (`Vietnamese`) — the setting
/// this reads has held all three over the life of the file, and a translation silently going into
/// the wrong language is not a failure anyone notices until the file is on disk.
///
/// Anything unrecognised comes back unchanged rather than being replaced by a default. A user who
/// types "Swiss German" means it, and turning that into "English" would be worse than passing it
/// through.
#[must_use]
pub fn name(language: &str) -> &str {
    let trimmed = language.trim();
    if trimmed.is_empty() {
        return trimmed;
    }
    let lower = trimmed.to_ascii_lowercase();

    if let Some((_, full)) = NAMES.iter().find(|(tag, _)| *tag == lower) {
        return full;
    }
    // `vi-VN`, `zh-CN`, `en-GB`: fall back to the primary subtag, but only after the full tag has
    // had its chance — `zh-Hant` is a different language from `zh` and must not be collapsed.
    if let Some(primary) = lower.split(['-', '_']).next()
        && let Some((_, full)) = NAMES.iter().find(|(tag, _)| *tag == primary)
    {
        return full;
    }
    trimmed
}

/// Whether this is a language the translation model was trained on.
///
/// Used to warn *before* a request rather than after: a translation into a language the model has
/// never seen comes back looking plausible and is not, which is the failure that costs the most to
/// discover late.
#[must_use]
pub fn known(language: &str) -> bool {
    let named = name(language);
    NAMES
        .iter()
        .any(|(tag, full)| *full == named || tag.eq_ignore_ascii_case(language.trim()))
}

/// Every language name, once, in alphabetical order.
///
/// `Norwegian` and `Chinese (Simplified)` each appear under two tags; a picker showing the same
/// language twice looks like a bug in the picker.
#[must_use]
pub fn all() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = NAMES.iter().map(|(_, full)| *full).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Writing systems, as far as "is this the language I asked for" needs to care.
///
/// Not a full Unicode script property — a coarse grouping, because the question is only ever
/// whether a reply landed in a completely different language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Script {
    Latin,
    Han,
    Kana,
    Hangul,
    Thai,
    Cyrillic,
    Arabic,
    Hebrew,
    Greek,
    Devanagari,
    Bengali,
    Tamil,
    Khmer,
    Lao,
    Myanmar,
}

fn script_of(c: char) -> Option<Script> {
    Some(match c as u32 {
        0x0041..=0x005A | 0x0061..=0x007A | 0x00C0..=0x024F | 0x1E00..=0x1EFF => Script::Latin,
        0x0370..=0x03FF | 0x1F00..=0x1FFF => Script::Greek,
        // Cyrillic, then Cyrillic Supplement — contiguous.
        0x0400..=0x052F => Script::Cyrillic,
        0x0590..=0x05FF => Script::Hebrew,
        0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF | 0xFB50..=0xFDFF => Script::Arabic,
        0x0900..=0x097F => Script::Devanagari,
        0x0980..=0x09FF => Script::Bengali,
        0x0B80..=0x0BFF => Script::Tamil,
        0x0E00..=0x0E7F => Script::Thai,
        0x0E80..=0x0EFF => Script::Lao,
        0x1000..=0x109F => Script::Myanmar,
        0x1780..=0x17FF => Script::Khmer,
        // Hiragana, then Katakana — contiguous, and the pair is what distinguishes Japanese
        // from Chinese here.
        0x3040..=0x30FF => Script::Kana,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF => Script::Han,
        0xAC00..=0xD7AF | 0x1100..=0x11FF => Script::Hangul,
        _ => return None,
    })
}

/// The script a translation into `language` is expected to be written in.
///
/// `None` for languages that share Latin with dozens of others — telling Spanish from Portuguese is
/// not something a character range can do, and pretending otherwise would reject correct answers.
fn expected(language: &str) -> Option<Script> {
    Some(match name(language) {
        "Japanese" => Script::Kana,
        "Chinese (Simplified)" | "Chinese (Traditional)" | "Cantonese" => Script::Han,
        "Korean" => Script::Hangul,
        "Thai" => Script::Thai,
        "Lao" => Script::Lao,
        "Khmer" => Script::Khmer,
        "Burmese" => Script::Myanmar,
        "Russian" | "Bulgarian" | "Kazakh" => Script::Cyrillic,
        "Arabic" | "Persian" | "Urdu" => Script::Arabic,
        "Hebrew" => Script::Hebrew,
        "Greek" => Script::Greek,
        "Hindi" => Script::Devanagari,
        "Bengali" => Script::Bengali,
        "Tamil" => Script::Tamil,
        _ => return None,
    })
}

/// Whether `text` could plausibly be a translation into `language`.
///
/// This exists because of a failure that nothing else in the pipeline can see. Asked for Japanese,
/// MiLMMT-46-1B returned one line of a three-line meeting in **Thai** — fluent, correct Thai,
/// written into a file labelled `ja`. It was not a malformed response, it did not fail to parse,
/// and it survived pinning the temperature to zero. Small translation models simply do this, and
/// the user who finds out is the one reading subtitles in a language they do not speak.
///
/// A script check is the cheapest thing that catches it, and deliberately the *only* thing it
/// catches. The rules, in order:
///
/// * Text containing the expected script is fine — Japanese with Han characters in it is Japanese.
/// * Kana in a Chinese answer is Japanese leaking through, and is the one case where sharing Han
///   would otherwise hide a wrong language.
/// * Any other non-Latin script, where the expected one is absent, is a different language.
/// * Latin is never evidence of anything. "OK", "API", "go-live" and every product name on earth
///   appear verbatim in correct Japanese, and rejecting a line for them would lose real
///   translations to catch nothing.
///
/// Languages with no distinctive script always pass. Telling French from Spanish is a job for a
/// language identifier, and a wrong guess there would throw away correct work.
#[must_use]
pub fn plausible(text: &str, language: &str) -> bool {
    let Some(want) = expected(language) else {
        return true;
    };
    let scripts: Vec<Script> = text.chars().filter_map(script_of).collect();
    if scripts.is_empty() {
        // Digits, punctuation, an emoji. Nothing to disagree with.
        return true;
    }
    // Before the general test, because Japanese contains Han: a Chinese answer written in kana and
    // kanji would otherwise pass on the kanji alone, which is the one wrong language this check
    // would let through.
    if want == Script::Han && scripts.contains(&Script::Kana) {
        return false;
    }
    if scripts.contains(&want) {
        return true;
    }
    if want == Script::Kana && scripts.contains(&Script::Han) {
        // Japanese written entirely in kanji is unusual but legal — a short noun phrase, a date, a
        // name. Kanji is the one script that cannot count as evidence against Japanese.
        return true;
    }
    !scripts.iter().any(|s| *s != Script::Latin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_becomes_the_name_a_model_understands() {
        assert_eq!(name("vi"), "Vietnamese");
        assert_eq!(name("ja"), "Japanese");
        assert_eq!(name("zh"), "Chinese (Simplified)");
    }

    #[test]
    fn a_name_is_already_a_name() {
        assert_eq!(name("Vietnamese"), "Vietnamese");
        assert_eq!(name("  English  "), "English");
    }

    #[test]
    fn a_regional_tag_falls_back_to_its_language() {
        assert_eq!(name("vi-VN"), "Vietnamese");
        assert_eq!(name("en_GB"), "English");
    }

    /// The one case a primary-subtag fallback gets wrong if it runs first. Traditional and
    /// Simplified are not the same output, and a Taiwanese user handed Simplified characters has
    /// been given the wrong language, not a variant of the right one.
    #[test]
    fn traditional_chinese_is_not_collapsed_into_simplified() {
        assert_eq!(name("zh-Hant"), "Chinese (Traditional)");
        assert_eq!(name("zh-hant"), "Chinese (Traditional)");
        assert_eq!(name("zh-CN"), "Chinese (Simplified)");
    }

    #[test]
    fn an_unknown_language_is_passed_through_rather_than_defaulted() {
        assert_eq!(name("Swiss German"), "Swiss German");
        assert_eq!(name("xx"), "xx");
    }

    #[test]
    fn an_empty_language_stays_empty() {
        assert_eq!(name(""), "");
        assert_eq!(name("   "), "");
    }

    #[test]
    fn the_supported_set_is_the_one_the_model_claims() {
        assert!(known("vi"));
        assert!(known("Japanese"));
        assert!(known("yue"));
        assert!(!known("Swiss German"));
        assert!(!known(""));
    }

    #[test]
    fn the_same_language_is_not_offered_twice() {
        let all = all();
        let mut sorted = all.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(all, sorted);
        assert!(all.contains(&"Vietnamese"));
        assert_eq!(all.iter().filter(|l| **l == "Norwegian").count(), 1);
    }

    /// The line that started this: asked for Japanese, the model answered in Thai.
    #[test]
    fn a_reply_in_the_wrong_language_is_not_plausible() {
        let thai = "โอเคค่ะ ฉันจะเลื่อนกำหนดไปเป็นวันศุกร์สัปดาห์หน้า";
        assert!(!plausible(thai, "ja"));
        assert!(!plausible(thai, "zh"));
        assert!(!plausible(thai, "ko"));
        assert!(plausible(thai, "th"), "Thai asked for is Thai delivered");
    }

    #[test]
    fn a_correct_reply_passes_in_every_script_we_check() {
        assert!(plausible("今夜、APIの仕様を確定します。", "ja"));
        assert!(plausible("今天下午我将API规格敲定。", "zh"));
        assert!(plausible("오늘 오후에 API 스펙을 확정하겠습니다.", "ko"));
        assert!(plausible("Я отправлю спецификацию клиенту.", "ru"));
        assert!(plausible("سأرسل المواصفات إلى العميل.", "ar"));
    }

    /// Han is shared, so Japanese would otherwise pass as Chinese. Kana is what gives it away.
    #[test]
    fn japanese_does_not_pass_as_chinese() {
        assert!(!plausible("今夜、APIの仕様を確定します。", "zh"));
    }

    /// Japanese written in kanji alone is unusual, not wrong — a date, a name, a noun phrase.
    #[test]
    fn japanese_written_only_in_kanji_is_still_japanese() {
        assert!(plausible("来週金曜日", "ja"));
    }

    /// The rule that stops this check costing more than it saves. These appear verbatim inside
    /// correct translations in every language, and rejecting a line for them would lose real work.
    #[test]
    fn latin_is_never_evidence_of_the_wrong_language() {
        assert!(plausible("OK", "ja"));
        assert!(plausible("API spec", "zh"));
        assert!(plausible("go-live", "ko"));
        assert!(plausible("2026-08-12 09:00", "ja"));
        assert!(plausible("", "ja"));
    }

    /// A character range cannot tell French from Spanish, and a wrong guess would throw away a
    /// correct translation. Languages with no distinctive script are not checked at all.
    #[test]
    fn languages_that_share_an_alphabet_are_not_second_guessed() {
        assert!(plausible("Je vais envoyer les specs.", "es"));
        assert!(plausible("bất kỳ câu nào", "en"));
        assert!(plausible("anything at all", "Swiss German"));
    }
}
