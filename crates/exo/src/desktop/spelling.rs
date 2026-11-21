//! macOS's spellchecker (`NSSpellChecker`), which checks compose's body: in the language System
//! Settings › Keyboard › Spelling sets, each paragraph's detected when that's `Automatic by
//! Language`; with the user's dictionary, which `Add to Dictionary` adds to (`learnWord:`); and a
//! spell document per body, which `Ignore Spelling` ignores a word in. Autocorrect and
//! autocomplete are macOS's too: its corrections (`correctionForWordRange:`), the user's text
//! replacements (System Settings › Keyboard › Text Replacements) while macOS makes them, and its
//! completions.

/// A spellchecker for one body: macOS's on the main thread (the app's), none elsewhere (tests,
/// which give their own) or off macOS.
pub(crate) fn engine() -> Option<Box<dyn exo_editor::Spelling>> {
    #[cfg(target_os = "macos")]
    return objc2::MainThreadMarker::new()
        .map(|_| Box::new(macos::Document::new()) as Box<dyn exo_editor::Spelling>);
    #[cfg(not(target_os = "macos"))]
    None
}

/// Whether macOS corrects spelling automatically (System Settings › Keyboard › Text Input ›
/// `Correct spelling automatically`): autocorrect's default until it's turned on or off in Exo.
pub(crate) fn corrects_automatically() -> bool {
    #[cfg(target_os = "macos")]
    return objc2_app_kit::NSSpellChecker::isAutomaticSpellingCorrectionEnabled();
    #[cfg(not(target_os = "macos"))]
    false
}

/// The languages whose greetings autocomplete offers: English, then the Mac's preferred
/// languages that have them (their first subtags), four at most.
pub(crate) fn greeting_languages() -> Vec<String> {
    #[cfg(target_os = "macos")]
    let preferred: Vec<String> = objc2_foundation::NSUserDefaults::standardUserDefaults()
        .stringArrayForKey(&objc2_foundation::NSString::from_str("AppleLanguages"))
        .map(|languages| {
            languages
                .iter()
                .map(|language| language.to_string())
                .collect()
        })
        .unwrap_or_default();
    #[cfg(not(target_os = "macos"))]
    let preferred: Vec<String> = Vec::new();
    let mut out = vec!["en".to_owned()];
    for language in preferred
        .iter()
        .filter_map(|language| language.split('-').next())
        .map(str::to_lowercase)
    {
        if exo_editor::GREETING_LANGUAGES.contains(&language.as_str()) && !out.contains(&language) {
            out.push(language);
        }
    }
    out.truncate(4);
    out
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ops::Range;

    use objc2_app_kit::NSSpellChecker;
    use objc2_foundation::{NSRange, NSString, NSTextCheckingType};

    /// One body's spell document (`uniqueSpellDocumentTag`), closed with it.
    pub(super) struct Document {
        tag: isize,
    }

    impl Document {
        pub(super) fn new() -> Self {
            Self {
                tag: NSSpellChecker::uniqueSpellDocumentTag(),
            }
        }
    }

    impl Drop for Document {
        fn drop(&mut self) {
            NSSpellChecker::sharedSpellChecker().closeSpellDocumentWithTag(self.tag);
        }
    }

    /// `text` as AppKit counts it, and the character each UTF-16 unit is in (one past the end
    /// too), to turn AppKit's ranges into the editor's.
    struct Units {
        string: objc2::rc::Retained<NSString>,
        chars: Vec<usize>,
    }

    impl Units {
        fn of(text: &str) -> Self {
            let mut chars = Vec::with_capacity(text.len() + 1);
            for (index, ch) in text.chars().enumerate() {
                chars.extend(std::iter::repeat_n(index, ch.len_utf16()));
            }
            chars.push(text.chars().count());
            Self {
                string: NSString::from_str(text),
                chars,
            }
        }

        fn chars(&self, range: NSRange) -> Range<usize> {
            let at = |unit: usize| {
                self.chars
                    .get(unit)
                    .copied()
                    .unwrap_or(self.chars[self.chars.len() - 1])
            };
            at(range.location)..at(range.location + range.length)
        }

        fn units(&self, range: Range<usize>) -> NSRange {
            let start = self
                .chars
                .iter()
                .position(|ch| *ch >= range.start)
                .unwrap_or(self.chars.len() - 1);
            let end = self
                .chars
                .iter()
                .position(|ch| *ch >= range.end)
                .unwrap_or(self.chars.len() - 1);
            NSRange::new(start, end - start)
        }
    }

    impl exo_editor::Spelling for Document {
        fn check(&mut self, text: &str) -> Vec<Range<usize>> {
            let units = Units::of(text);
            let checker = NSSpellChecker::sharedSpellChecker();
            // SAFETY: no options, and no orthography or word count asked for back.
            let results = unsafe {
                checker
                    .checkString_range_types_options_inSpellDocumentWithTag_orthography_wordCount(
                        &units.string,
                        NSRange::new(0, units.string.length()),
                        NSTextCheckingType::Spelling.0,
                        None,
                        self.tag,
                        None,
                        std::ptr::null_mut(),
                    )
            };
            results
                .iter()
                .filter(|result| result.resultType() == NSTextCheckingType::Spelling)
                .map(|result| units.chars(result.range()))
                .filter(|range| !range.is_empty())
                .collect()
        }

        fn guesses(&mut self, text: &str, word: Range<usize>) -> Vec<String> {
            let units = Units::of(text);
            let guesses = NSSpellChecker::sharedSpellChecker()
                .guessesForWordRange_inString_language_inSpellDocumentWithTag(
                    units.units(word),
                    &units.string,
                    None,
                    self.tag,
                );
            guesses
                .map(|guesses| guesses.iter().map(|guess| guess.to_string()).collect())
                .unwrap_or_default()
        }

        fn learn(&mut self, word: &str) {
            NSSpellChecker::sharedSpellChecker().learnWord(&NSString::from_str(word));
        }

        fn ignore(&mut self, word: &str) {
            NSSpellChecker::sharedSpellChecker()
                .ignoreWord_inSpellDocumentWithTag(&NSString::from_str(word), self.tag);
        }

        fn replacement(&mut self, word: &str) -> Option<String> {
            if !NSSpellChecker::isAutomaticTextReplacementEnabled() {
                return None;
            }
            NSSpellChecker::sharedSpellChecker()
                .userReplacementsDictionary()
                .objectForKey(&NSString::from_str(word))
                .map(|with| with.to_string())
        }

        fn correction(&mut self, text: &str, word: Range<usize>) -> Option<String> {
            let units = Units::of(text);
            let checker = NSSpellChecker::sharedSpellChecker();
            let range = units.units(word);
            // The paragraph's language, as `Automatic by Language` detects it, else the one set.
            let language = checker
                .languageForWordRange_inString_orthography(range, &units.string, None)
                .unwrap_or_else(|| checker.language());
            checker
                .correctionForWordRange_inString_language_inSpellDocumentWithTag(
                    range,
                    &units.string,
                    &language,
                    self.tag,
                )
                .map(|with| with.to_string())
        }

        fn completions(&mut self, text: &str, word: Range<usize>) -> Vec<String> {
            let units = Units::of(text);
            let completions = NSSpellChecker::sharedSpellChecker()
                .completionsForPartialWordRange_inString_language_inSpellDocumentWithTag(
                    units.units(word),
                    &units.string,
                    None,
                    self.tag,
                );
            completions
                .map(|words| words.iter().map(|word| word.to_string()).collect())
                .unwrap_or_default()
        }
    }

    #[cfg(test)]
    mod tests {
        use exo_editor::Spelling as _;

        use super::*;

        // Reaches macOS's spell server (outside the sandbox, as the suite runs). Never learns a
        // word: that would change the user's dictionary.
        #[test]
        fn macos_marks_a_misspelling_and_guesses_its_correction() {
            let mut document = Document::new();
            let text = "Café: I saw teh cat 👋 yesterdy";
            let (teh, yesterdy) = (12..15, 22..30);
            assert_eq!(
                document.check(text),
                [teh.clone(), yesterdy.clone()],
                "character ranges, past the emoji's two UTF-16 units"
            );
            assert_eq!(
                document
                    .guesses(text, teh.clone())
                    .first()
                    .map(String::as_str),
                Some("the")
            );
            document.ignore("teh");
            assert_eq!(
                document.check(text),
                std::slice::from_ref(&yesterdy),
                "ignored in this document"
            );
            assert_eq!(
                Document::new().check(text),
                [teh, yesterdy],
                "and only there"
            );
        }

        // macOS's autocorrection and completion, as autocorrect and autocomplete ask them.
        #[test]
        fn macos_corrects_a_misspelling_and_completes_a_word() {
            let mut document = Document::new();
            assert_eq!(
                document.correction("I saw teh cat", 6..9).as_deref(),
                Some("the")
            );
            assert_eq!(
                document.correction("I saw the cat", 6..9),
                None,
                "nothing to correct"
            );
            let completions = document.completions("I saw it yesterd", 9..16);
            assert_eq!(
                completions.first().map(String::as_str),
                Some("yesterday"),
                "{completions:?}"
            );
        }
    }
}
