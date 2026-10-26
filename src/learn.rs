//! Learn vocabulary from the corrections a user makes to pasted text.
//!
//! Given the text Murmur pasted and what the field contains afterwards, find small word
//! replacements that look like speech-recognition mishearings ("Cloud Code" -> "Claude Code")
//! and return them as dictionary candidates.

use crate::types::LearnedTerm;
use once_cell::sync::Lazy;
use std::collections::HashSet;

/// Max words on either side of a replaced span.
const MAX_SPAN_WORDS: usize = 3;
/// If more than this fraction of the pasted words changed, the user rewrote rather than corrected.
const MAX_CHANGED_FRACTION: f32 = 0.4;
/// The rewrite check only applies from this many pasted words (a one-word paste is all "changed").
const MIN_WORDS_FOR_REWRITE_CHECK: usize = 4;
const MAX_TERM_CHARS: usize = 40;

static COMMON: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    "the a an and or but if so to of in on at by for with from as is are was were be been am do does did \
     have has had it its this that these those there their theirs theyre they them he she him her his hers \
     we us our you your youre yours i me my mine not no yes can could will would should shall may might must \
     what when where which who whom whose why how all any some each every more most other such than then \
     too very just also only about into over after before up down out off again here now one two get got \
     go going make made know like want need see say said let lets well back even new good time dont cant \
     wont im ive id thats whats theres hes shes were weve ill then than though through while because until \
     your whether either neither both own same much many few less little long still sure okay ok"
        .split_whitespace()
        .collect()
});

/// Days and months look alike in spelling but swapping one for another is a content change, not a mishearing.
const CALENDAR_WORDS: &[&str] = &[
    "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday", "january", "february",
    "march", "april", "may", "june", "july", "august", "september", "october", "november", "december",
];

#[derive(Debug, Clone)]
struct Tok {
    /// Original text with punctuation trimmed from both ends (casing kept).
    text: String,
    /// Lowercased, letters and digits only.
    key: String,
    /// The word ended with sentence-final punctuation in the source text.
    ends_sentence: bool,
}

fn tokenize(s: &str) -> Vec<Tok> {
    s.split_whitespace()
        .filter_map(|raw| {
            let text = raw.trim_matches(|c: char| !c.is_alphanumeric()).to_string();
            if text.is_empty() {
                return None;
            }
            let key: String = text.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect();
            let ends_sentence = raw.trim_end_matches(['"', '\'', ')', ']', '”', '’']).ends_with(['.', '!', '?', '…']);
            Some(Tok { text, key, ends_sentence })
        })
        .collect()
}

#[derive(Debug)]
struct Span {
    old: Vec<Tok>,
    new: Vec<Tok>,
    /// True when the first old word starts a sentence.
    at_sentence_start: bool,
}

/// Word-level diff via LCS. Returns replaced spans and the number of pasted words that changed.
fn diff(a: &[Tok], b: &[Tok]) -> (Vec<Span>, usize) {
    let (n, m) = (a.len(), b.len());
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i].text == b[j].text { dp[i + 1][j + 1] + 1 } else { dp[i + 1][j].max(dp[i][j + 1]) };
        }
    }
    let mut spans = Vec::new();
    let mut changed = 0;
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i].text == b[j].text {
            i += 1;
            j += 1;
            continue;
        }
        let (si, sj) = (i, j);
        while (i < n || j < m) && !(i < n && j < m && a[i].text == b[j].text) {
            if i < n && (j >= m || dp[i + 1][j] >= dp[i][j + 1]) {
                i += 1;
            } else {
                j += 1;
            }
        }
        changed += (i - si).max(j - sj);
        if i > si && j > sj {
            spans.push(Span {
                old: a[si..i].to_vec(),
                new: b[sj..j].to_vec(),
                at_sentence_start: si == 0 || a[si - 1].ends_sentence,
            });
        }
    }
    (spans, changed)
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j] + (ca != cb) as usize).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

fn letters(toks: &[Tok]) -> Vec<char> {
    toks.iter().flat_map(|t| t.text.chars()).filter(|c| c.is_alphabetic()).flat_map(|c| c.to_lowercase()).collect()
}

fn soundex(word: &str) -> String {
    let code = |c: char| match c {
        'b' | 'f' | 'p' | 'v' => '1',
        'c' | 'g' | 'j' | 'k' | 'q' | 's' | 'x' | 'z' => '2',
        'd' | 't' => '3',
        'l' => '4',
        'm' | 'n' => '5',
        'r' => '6',
        _ => '0',
    };
    let mut chars = word.chars().filter(|c| c.is_ascii_alphabetic()).map(|c| c.to_ascii_lowercase());
    let Some(first) = chars.next() else { return String::new() };
    let mut out = String::from(first.to_ascii_uppercase());
    let mut last = code(first);
    for c in chars {
        let d = code(c);
        if d != '0' && d != last {
            out.push(d);
        }
        // h and w do not separate equal codes.
        if c != 'h' && c != 'w' {
            last = d;
        }
        if out.len() == 4 {
            break;
        }
    }
    while out.len() < 4 {
        out.push('0');
    }
    out
}

fn sounds_alike(old: &[Tok], new: &[Tok]) -> bool {
    let (a, b) = (letters(old), letters(new));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let max = a.len().max(b.len());
    let ratio = 1.0 - levenshtein(&a, &b) as f32 / max as f32;
    if ratio >= 0.5 {
        return true;
    }
    if a[0] == b[0] && a.len().min(b.len()) as f32 / max as f32 >= 0.5 {
        return true;
    }
    let (sa, sb): (String, String) = (a.iter().collect(), b.iter().collect());
    let (sa, sb) = (soundex(&sa), soundex(&sb));
    !sa.is_empty() && sa == sb
}

fn has_upper(s: &str) -> bool {
    s.chars().any(|c| c.is_uppercase())
}

fn is_capitalized(t: &Tok) -> bool {
    t.text.chars().next().is_some_and(|c| c.is_uppercase())
}

fn accept(span: &Span) -> bool {
    let (old, new) = (&span.old, &span.new);
    if old.len() > MAX_SPAN_WORDS || new.len() > MAX_SPAN_WORDS {
        return false;
    }
    let new_text = new.iter().map(|t| t.text.as_str()).collect::<Vec<_>>().join(" ");
    if new_text.chars().count() > MAX_TERM_CHARS {
        return false;
    }
    if new.iter().any(|t| t.text.chars().all(|c| c.is_ascii_digit())) {
        return false;
    }
    let old_keys: Vec<&str> = old.iter().map(|t| t.key.as_str()).collect();
    let new_keys: Vec<&str> = new.iter().map(|t| t.key.as_str()).collect();
    // A word that is only common grammar is not a term, unless the user capitalized it.
    let all_common = new_keys.iter().all(|k| COMMON.contains(k));
    if old_keys == new_keys {
        // Case-only change: only a newly capitalized word (proper noun, product, acronym) counts.
        let old_text = old.iter().map(|t| t.text.as_str()).collect::<Vec<_>>().join(" ");
        if !has_upper(&new_text) || all_common {
            return false;
        }
        let interior_upper = new.iter().any(|t| t.text.chars().skip(1).any(|c| c.is_uppercase()));
        let gained_upper = new_text.chars().filter(|c| c.is_uppercase()).count()
            > old_text.chars().filter(|c| c.is_uppercase()).count();
        return gained_upper && (!span.at_sentence_start || interior_upper);
    }
    if old_keys.iter().chain(&new_keys).all(|k| CALENDAR_WORDS.contains(k)) {
        return false;
    }
    if all_common && !new.iter().any(is_capitalized) {
        return false;
    }
    sounds_alike(old, new)
}

/// Terms the user corrected in `pasted` to produce `edited`. `word` is the corrected form as typed,
/// `heard_as` what Murmur had pasted. Empty when the edit is a rewrite or nothing looks like a mishearing.
pub fn learn_from_edit(pasted: &str, edited: &str) -> Vec<LearnedTerm> {
    let (a, b) = (tokenize(pasted), tokenize(edited));
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let (spans, changed) = diff(&a, &b);
    if a.len() >= MIN_WORDS_FOR_REWRITE_CHECK && changed as f32 > MAX_CHANGED_FRACTION * a.len() as f32 {
        return Vec::new();
    }
    let join = |t: &[Tok]| t.iter().map(|t| t.text.as_str()).collect::<Vec<_>>().join(" ");
    let mut out: Vec<LearnedTerm> = Vec::new();
    for span in spans.iter().filter(|s| accept(s)) {
        let term = LearnedTerm { word: join(&span.new), heard_as: join(&span.old) };
        if !out.iter().any(|t| t.word.eq_ignore_ascii_case(&term.word)) {
            out.push(term);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(word: &str, heard: &str) -> LearnedTerm {
        LearnedTerm { word: word.into(), heard_as: heard.into() }
    }

    #[test]
    fn cloud_code_to_claude_code() {
        let r = learn_from_edit(
            "Can you open Cloud Code and run the tests?",
            "Can you open Claude Code and run the tests?",
        );
        assert_eq!(r, vec![t("Claude", "Cloud")]);
    }

    #[test]
    fn two_word_span_is_kept_whole() {
        let r = learn_from_edit("Please open Cloud Code now", "Please open Claude Codex now");
        assert_eq!(r, vec![t("Claude Codex", "Cloud Code")]);
    }

    #[test]
    fn lowercase_to_proper_noun_mid_sentence() {
        assert_eq!(learn_from_edit("ask cloud about it", "ask Claude about it"), vec![t("Claude", "cloud")]);
        assert_eq!(learn_from_edit("ask claude about it", "ask Claude about it"), vec![t("Claude", "claude")]);
        assert_eq!(learn_from_edit("I use github daily", "I use GitHub daily"), vec![t("GitHub", "github")]);
    }

    #[test]
    fn common_word_swap_rejected() {
        assert!(learn_from_edit("I left it over there on the desk", "I left it over their on the desk").is_empty());
    }

    #[test]
    fn dissimilar_words_rejected() {
        assert!(learn_from_edit("See you on Tuesday at the office", "See you on Wednesday at the office").is_empty());
    }

    #[test]
    fn full_rewrite_rejected() {
        assert!(learn_from_edit("Let's meet tomorrow for lunch", "Send the report to finance").is_empty());
        assert!(learn_from_edit(
            "please send the report to Claude before lunch today",
            "kindly forward that document Claude after dinner tonight",
        )
        .is_empty());
    }

    #[test]
    fn name_typo_fix_accepted() {
        assert_eq!(learn_from_edit("Tell Pria I said hello", "Tell Priya I said hello"), vec![t("Priya", "Pria")]);
    }

    #[test]
    fn punctuation_only_rejected() {
        assert!(learn_from_edit("hello world how are you", "Hello, world. How are you?").is_empty());
        assert!(learn_from_edit("Can you open it", "Can you open it!").is_empty());
    }

    #[test]
    fn case_only_on_common_word_rejected() {
        assert!(learn_from_edit("The tests are passing", "the tests are passing").is_empty());
        assert!(learn_from_edit("we saw The tests pass", "we saw the tests pass").is_empty());
        // Capitalizing a sentence start is not a proper noun either.
        assert!(learn_from_edit("it works. cloud is fine", "It works. Cloud is fine").is_empty());
    }

    #[test]
    fn multiple_corrections_in_one_edit() {
        let r = learn_from_edit(
            "Please ask cloud about the meeting with pria and the open ai team at the office today",
            "Please ask Claude about the meeting with Priya and the OpenAI team at the office today",
        );
        assert_eq!(r, vec![t("Claude", "cloud"), t("Priya", "pria"), t("OpenAI", "open ai")]);
    }

    #[test]
    fn pure_insertions_and_deletions_ignored() {
        assert!(learn_from_edit("send it to Bob today", "send it to Bob today please").is_empty());
        assert!(learn_from_edit("send it to Bob today please", "send it to Bob today").is_empty());
    }

    #[test]
    fn digits_and_long_terms_rejected() {
        assert!(learn_from_edit("I have to tickets for you", "I have 2 tickets for you").is_empty());
        let long = "Supercalifragilisticexpialidocious".repeat(2);
        assert!(learn_from_edit("say Supercalifragilisticexpialidocius now please ok", &format!("say {long} now please ok")).is_empty());
    }

    #[test]
    fn long_spans_rejected_and_duplicates_collapsed() {
        assert!(learn_from_edit("we met at the old red barn today ok", "we met at new blue big green house today ok").is_empty());
        let r = learn_from_edit("cloud is great and cloud is fast and fun to use", "Claude is great and Claude is fast and fun to use");
        assert_eq!(r, vec![t("Claude", "cloud")]);
    }

    #[test]
    fn empty_and_identical_inputs() {
        assert!(learn_from_edit("", "").is_empty());
        assert!(learn_from_edit("same text", "same text").is_empty());
        assert!(learn_from_edit("something", "").is_empty());
    }

    #[test]
    fn soundex_basics() {
        assert_eq!(soundex("Robert"), "R163");
        assert_eq!(soundex("Rupert"), "R163");
        assert_eq!(soundex("Tymczak"), "T522");
    }
}
