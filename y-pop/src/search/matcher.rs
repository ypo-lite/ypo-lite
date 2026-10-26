//! The fuzzy matcher: one query word against one piece of text.
//!
//! Both sides are **folded** first ([`fold`]: lower case, accents removed),
//! the text once when it is indexed and the query once per keystroke, so the
//! matcher itself compares plain `char`s and allocates nothing per call.
//! Folding is one `char` to one `char`, which keeps a character position in
//! the folded text equal to the same position in the original — what a
//! result row needs to emphasize the matched letters.
//!
//! # Scoring
//!
//! Spec §2.8's order — whole-name prefix > word-start prefix > acronym >
//! contiguous substring > scattered characters — is made literal: each kind
//! of match has its own band of the `0..=1000` scale, so a better kind
//! always beats a worse one whatever the text lengths. Inside a band a
//! shorter text wins (a query covers more of it).
//!
//! | Kind | Band | Example, query `out` |
//! |---|---|---|
//! | whole-text prefix | 850–900 | **Out**look |
//! | word-start prefix | 700–750 | Microsoft **Out**look |
//! | acronym (word initials) | 550–600 | **O**pen **U**tility **T**ool |
//! | contiguous substring | 400–450 | Sh**out**cast |
//! | scattered | 1–300 | **O**f**u**ll **t**ext |
//!
//! Scattered matches are scored by a small dynamic program: a letter that
//! starts a word or follows the previous match scores more, a gap costs a
//! little (more as it grows).

/// `text` without the menu mnemonic markers of a tray menu's names: `&Word`
/// is `Word` (the `&` only underlines the letter after it in a menu), `&&` is a
/// literal `&`. Search shows, indexes and emphasizes this form: drawn as it is,
/// the `&` would swallow itself and underline the next letter, and a whole-name
/// prefix match would never see the first letter.
pub fn plain(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                plain.push('&');
                chars.next();
            }
        } else {
            plain.push(c);
        }
    }
    plain
}

/// `text` lower-cased with accents removed, one `char` per `char`.
pub fn fold(text: &str) -> String {
    text.chars().map(fold_char).collect()
}

/// Lower case, and the base letter of the common accented Latin ones. A
/// letter that folds to two (`ß`, `æ`, `œ`) folds to the first, to keep the
/// one-to-one mapping.
pub fn fold_char(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    // `to_lowercase` can yield several chars (`İ`); only the first is kept.
    match c.to_lowercase().next().unwrap_or(c) {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' | 'æ' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' | 'ħ' => 'h',
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' | 'œ' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' | 'ß' => 's',
        'ţ' | 'ť' | 'ŧ' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        other => other,
    }
}

/// A letter or digit that begins a word: the first character, or one
/// following a space, `\`, `-`, `(`, `›`… anything that is not alphanumeric.
fn word_start(text: &[char], at: usize) -> bool {
    text[at].is_alphanumeric() && (at == 0 || !text[at - 1].is_alphanumeric())
}

const PREFIX: u32 = 900;
const WORD_PREFIX: u32 = 750;
const ACRONYM: u32 = 600;
const SUBSTRING: u32 = 450;
const SCATTERED_FLOOR: u32 = 100;
const SCATTERED_SPAN: u32 = 200;
/// How much of the text left over after the query can lower a score.
const MAX_LENGTH_PENALTY: usize = 50;

/// Reusable scratch space, so matching allocates nothing per call once it
/// has grown to the longest text seen.
#[derive(Default)]
pub struct Matcher {
    text: Vec<char>,
    best: Vec<i32>,
    from: Vec<u16>,
}

impl Matcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// How well `query` matches `text` (both folded), `0..=1000`, or `None`
    /// if `query` is not a subsequence of `text`.
    pub fn score(&mut self, query: &[char], text: &str) -> Option<u32> {
        self.run(query, text, None)
    }

    /// [`Matcher::score`], also filling `positions` with the character
    /// indices of `text` the query matched, ascending.
    pub fn positions(
        &mut self,
        query: &[char],
        text: &str,
        positions: &mut Vec<usize>,
    ) -> Option<u32> {
        self.run(query, text, Some(positions))
    }

    fn run(&mut self, query: &[char], text: &str, mut out: Option<&mut Vec<usize>>) -> Option<u32> {
        if let Some(out) = out.as_deref_mut() {
            out.clear();
        }
        self.text.clear();
        self.text.extend(text.chars());
        let (m, n) = (query.len(), self.text.len());
        if m == 0 || m > n {
            return None;
        }

        // Cheap rejection first: most entries do not match at all.
        let mut next = 0;
        for &c in &self.text {
            if c == query[next] {
                next += 1;
                if next == m {
                    break;
                }
            }
        }
        if next < m {
            return None;
        }

        let penalty = (n - m).min(MAX_LENGTH_PENALTY) as u32;
        let mut mark = |range: std::ops::Range<usize>| {
            if let Some(out) = out.as_deref_mut() {
                out.extend(range);
            }
        };

        if self.text[..m] == *query {
            mark(0..m);
            return Some(PREFIX - penalty);
        }
        for at in 1..=n - m {
            if word_start(&self.text, at) && self.text[at..at + m] == *query {
                mark(at..at + m);
                return Some(WORD_PREFIX - penalty);
            }
        }

        if m >= 2 {
            let mut initials = Vec::new();
            let mut next = 0;
            for at in 0..n {
                if next < m && self.text[at] == query[next] && word_start(&self.text, at) {
                    initials.push(at);
                    next += 1;
                }
            }
            if next == m {
                if let Some(out) = out.as_deref_mut() {
                    out.extend(initials);
                }
                return Some(ACRONYM - penalty);
            }
        }

        if let Some(at) = (0..=n - m).find(|&at| self.text[at..at + m] == *query) {
            if let Some(out) = out.as_deref_mut() {
                out.extend(at..at + m);
            }
            return Some(SUBSTRING - penalty);
        }

        let raw = scatter(&self.text, query, &mut self.best, &mut self.from, out);
        let top = (m as i32) * 48 + (m as i32 - 1) * 18;
        let scaled = SCATTERED_SPAN as i32 * raw.clamp(0, top) / top.max(1);
        Some(
            (SCATTERED_FLOOR + scaled as u32)
                .saturating_sub(penalty)
                .max(1),
        )
    }
}

/// The best scattered alignment of `query` in `text`, as a raw score, and
/// its positions if asked. The caller has checked `query` is a subsequence.
///
/// `best[i * n + j]` is the best score with `query[i]` matched at `text[j]`
/// and everything before it matched too; `from` remembers where the previous
/// letter was. A match costs nothing to start; one at a word start scores
/// more, one right after the previous match scores more, and a gap of `g`
/// skipped letters costs `g + 2`. The gap transition keeps a running maximum
/// of `best[i - 1][k] + k` instead of looking at every `k`, which makes the
/// whole thing `O(query * text)`.
fn scatter(
    text: &[char],
    query: &[char],
    best: &mut Vec<i32>,
    from: &mut Vec<u16>,
    out: Option<&mut Vec<usize>>,
) -> i32 {
    const NONE: i32 = i32::MIN / 2;
    let (m, n) = (query.len(), text.len());
    best.clear();
    best.resize(m * n, NONE);
    from.clear();
    from.resize(m * n, 0);

    for i in 0..m {
        let mut run = NONE;
        let mut run_at = 0u16;
        for j in 0..n {
            if i > 0 && j >= 2 {
                let previous = best[(i - 1) * n + j - 2];
                if previous > NONE && previous + (j as i32 - 2) > run {
                    run = previous + (j as i32 - 2);
                    run_at = (j - 2) as u16;
                }
            }
            if text[j] != query[i] {
                continue;
            }
            let start = 16 + if word_start(text, j) { 24 } else { 0 } + if j == 0 { 8 } else { 0 };
            if i == 0 {
                best[j] = start - (j as i32).min(8);
                continue;
            }

            let mut chosen = NONE;
            let mut chosen_from = 0u16;
            if j >= 1 {
                let previous = best[(i - 1) * n + j - 1];
                if previous > NONE {
                    chosen = previous + 18;
                    chosen_from = (j - 1) as u16;
                }
            }
            if run > NONE && run - j as i32 - 1 > chosen {
                chosen = run - j as i32 - 1;
                chosen_from = run_at;
            }
            if chosen > NONE {
                best[i * n + j] = start + chosen;
                from[i * n + j] = chosen_from;
            }
        }
    }

    let last = (m - 1) * n;
    let (mut end, mut raw) = (0, NONE);
    for j in 0..n {
        if best[last + j] > raw {
            (end, raw) = (j, best[last + j]);
        }
    }

    if let Some(out) = out {
        let mut at = end;
        out.push(at);
        for i in (1..m).rev() {
            at = from[i * n + at] as usize;
            out.push(at);
        }
        out.reverse();
    }
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(text: &str) -> Vec<char> {
        fold(text).chars().collect()
    }

    #[test]
    fn mnemonic_markers_are_not_part_of_a_name() {
        assert_eq!(plain("&Word"), "Word");
        assert_eq!(plain("Publ&isher"), "Publisher");
        assert_eq!(plain("R&&D"), "R&D");
        assert_eq!(plain("Trailing &"), "Trailing ");
        assert_eq!(plain("Plain"), "Plain");
        assert_eq!(plain("y-pop › &Office"), "y-pop › Office");
    }

    fn score(query: &str, text: &str) -> Option<u32> {
        Matcher::new().score(&chars(query), &fold(text))
    }

    fn positions(query: &str, text: &str) -> Vec<usize> {
        let mut out = Vec::new();
        Matcher::new()
            .positions(&chars(query), &fold(text), &mut out)
            .expect("a match");
        out
    }

    #[test]
    fn folding_is_one_char_to_one_char() {
        for text in [
            "Café Réseau",
            "ÀÉÎÕÜ",
            "Straße",
            "Łódź",
            "日本語",
            "İstanbul",
        ] {
            assert_eq!(fold(text).chars().count(), text.chars().count(), "{text}");
        }
        assert_eq!(fold("Café Réseau"), "cafe reseau");
        assert_eq!(fold("ÀÉÎÕÜ"), "aeiou");
    }

    #[test]
    fn kinds_of_match_rank_in_the_documented_order() {
        let prefix = score("out", "Outlook").unwrap();
        let word = score("out", "Microsoft Outlook").unwrap();
        let acronym = score("out", "Open Utility Tool").unwrap();
        let substring = score("out", "Shoutcast").unwrap();
        let scattered = score("out", "Of full text").unwrap();
        assert!(prefix > word, "{prefix} > {word}");
        assert!(word > acronym, "{word} > {acronym}");
        assert!(acronym > substring, "{acronym} > {substring}");
        assert!(substring > scattered, "{substring} > {scattered}");
        assert!(scattered > 0);
    }

    #[test]
    fn a_better_kind_wins_whatever_the_lengths() {
        let long_prefix = score("a", &format!("a{}", "x".repeat(200))).unwrap();
        let short_word = score("a", "x a").unwrap();
        assert!(long_prefix > short_word);
    }

    #[test]
    fn inside_a_kind_the_shorter_text_wins() {
        assert!(score("note", "Notes").unwrap() > score("note", "Notepad++").unwrap());
        assert!(
            score("note", "Notepad++").unwrap() > score("note", "Notes and reminders").unwrap()
        );
    }

    #[test]
    fn no_match_when_not_a_subsequence() {
        assert_eq!(score("xyz", "Outlook"), None);
        assert_eq!(score("ktl", "Outlook"), None, "letters out of order");
        assert_eq!(score("outlookk", "Outlook"), None, "longer than the text");
        assert_eq!(score("", "Outlook"), None, "an empty query matches nothing");
    }

    #[test]
    fn accents_and_case_do_not_matter() {
        assert!(score("CAFE", "Café").is_some());
        assert!(score("café", "CAFE").is_some());
        assert_eq!(
            score("reseau", "Café Réseau"),
            score("RÉSEAU", "café reseau")
        );
    }

    #[test]
    fn positions_of_each_kind() {
        assert_eq!(positions("out", "Outlook"), [0, 1, 2]);
        assert_eq!(positions("out", "Microsoft Outlook"), [10, 11, 12]);
        assert_eq!(positions("out", "Open Utility Tool"), [0, 5, 13]);
        assert_eq!(positions("out", "Shoutcast"), [2, 3, 4]);
    }

    #[test]
    fn scattered_positions_prefer_word_starts_and_runs() {
        // Initials of "Command line".
        assert_eq!(positions("cl", "Command line"), [0, 8]);

        // Scattered: the `t` that starts "team" beats the first `t` found
        // left to right, even though its `m` is further away.
        assert_eq!(positions("tm", "atom team"), [5, 8]);
        // A word start, then the rest of the word.
        assert_eq!(positions("dsk", "Remote Desktop"), [7, 9, 10]);

        let found = positions("rmd", "Remote Desktop");
        assert_eq!(found.len(), 3);
        assert!(found.windows(2).all(|pair| pair[0] < pair[1]), "{found:?}");
    }

    #[test]
    fn positions_are_cleared_when_nothing_matches() {
        let mut out = vec![1, 2, 3];
        assert_eq!(
            Matcher::new().positions(&chars("zzz"), "abc", &mut out),
            None
        );
        assert!(out.is_empty());
    }

    #[test]
    fn a_word_boundary_is_any_non_alphanumeric() {
        assert!(score("tool", "my-tool").unwrap() > score("tool", "mytool").unwrap());
        assert_eq!(score("tool", "c:\\tool"), score("tool", "my tool"));
        assert_eq!(positions("rem", "Work › Remote"), [7, 8, 9]);
    }

    #[test]
    fn the_matcher_is_reusable_across_texts_of_any_length() {
        let mut matcher = Matcher::new();
        let long = "x".repeat(500) + "needle";
        assert!(matcher.score(&chars("needle"), &fold(&long)).is_some());
        assert!(matcher.score(&chars("ab"), "a b").is_some());
        assert!(matcher.score(&chars("zz"), "a b").is_none());
    }
}
