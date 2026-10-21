//! Typo tolerance for search. Titles are matched in memory: uncorrected tiers
//! (exact, prefix, word-start, substring) and, when allowed, fzf-style
//! subsequences. Body words are corrected through the FTS vocabulary: whole
//! words by a deletion index, the last (still typed) word by a walk over the
//! sorted vocabulary. Only query words missing from the FTS vocabulary are
//! corrected, and only they may use edits in titles (`index.rs`,
//! `missing_words`). Result counts never decide whether correction runs.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::ops::Range;

use crate::types::SearchHit;

/// Candidate corrections kept per query word.
const MAX_CANDIDATES: usize = 8;
/// Added to a body hit's bm25 (lower is better) for each edit it needed.
pub(crate) const EDIT_PENALTY: f64 = 4.0;
/// Taken from a title's fuzzy score for each edit it needed.
const TITLE_EDIT_PENALTY: i32 = 12;

/// Title tiers, best first. Tiers below `FUZZY` are uncorrected matches.
const EXACT: u8 = 0;
const PREFIX: u8 = 1;
const WORD_START: u8 = 2;
const SUBSTRING: u8 = 3;
const FUZZY: u8 = 4;

/// Edits allowed for a query word: none up to three letters, one up to
/// seven, two beyond. Title matching allows at most one, see `match_word`.
pub(crate) fn max_edits(len: usize) -> u32 {
    match len {
        0..=3 => 0,
        4..=7 => 1,
        _ => 2,
    }
}

/// Lowercase with common Latin diacritics removed, as the FTS tokenizer folds.
pub(crate) fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        for l in c.to_lowercase() {
            out.push(unaccent(l));
        }
    }
    out
}

fn unaccent(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        _ => c,
    }
}

/// The query's words, plain and split like `fts_query` splits them.
pub(crate) fn query_words(query: &str) -> Vec<String> {
    plain(query)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// A live, non-template page as the title matcher sees it.
pub(crate) struct TitleEntry {
    pub id: String,
    pub title: String,
    plain: String,
    ascii_mask: u128,
    letters: [u8; 26],
    chars: u32,
    updated_ms: i64,
}

impl TitleEntry {
    pub fn new(id: String, title: String, updated_ms: i64) -> TitleEntry {
        let plain = plain(&title);
        let ascii_mask = ascii_mask(plain.chars());
        let mut letters = [0u8; 26];
        for c in plain.bytes().filter(u8::is_ascii_lowercase) {
            let n = usize::from(c - b'a');
            letters[n] = letters[n].saturating_add(1);
        }
        let chars = plain.chars().count() as u32;
        TitleEntry {
            id,
            title,
            plain,
            ascii_mask,
            letters,
            chars,
            updated_ms,
        }
    }
}

/// Uncorrected tier of a title against the plain query words: the whole query
/// equals the title, starts it, starts a title word run (each query word
/// starts a later title word, `week rev` for `Weekly review`), or occurs as a
/// contiguous substring. `None` when no uncorrected form matches.
fn uncorrected_tier(query: &str, words: &[String], title: &str) -> Option<u8> {
    if title == query {
        Some(EXACT)
    } else if title.starts_with(query) {
        Some(PREFIX)
    } else if word_starts(words, title) {
        Some(WORD_START)
    } else if title.contains(query) {
        Some(SUBSTRING)
    } else {
        None
    }
}

fn word_starts(words: &[String], title: &str) -> bool {
    let mut title_words = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty());
    words
        .iter()
        .all(|q| title_words.any(|t| t.starts_with(q.as_str())))
}

/// Greedy in-order match of `word` in `title`, skipping title characters.
/// `wild` is a query position that matches any one title character (a
/// substitution). Each match earns points, consecutive and word-start matches
/// more; each skipped character costs one point.
fn subsequence(word: &[char], wild: Option<usize>, title: &str) -> Option<i32> {
    if title.is_ascii() {
        subsequence_chars(word, wild, title.bytes().map(char::from))
    } else {
        subsequence_chars(word, wild, title.chars())
    }
}

fn subsequence_chars(
    word: &[char],
    wild: Option<usize>,
    title: impl Iterator<Item = char>,
) -> Option<i32> {
    let mut score = 0;
    let mut next = 0;
    let mut last: Option<usize> = None;
    let mut prev = ' ';
    for (pos, c) in title.enumerate() {
        let Some(&want) = word.get(next) else {
            break;
        };
        let wildcard = wild == Some(next);
        if wildcard || c == want {
            if let Some(l) = last {
                score -= (pos - l - 1) as i32;
                if l + 1 == pos {
                    score += 8;
                }
            }
            score += if wildcard { 4 } else { 16 };
            if !prev.is_alphanumeric() {
                score += 10;
            }
            last = Some(pos);
            next += 1;
        }
        prev = c;
    }
    (next == word.len()).then_some(score)
}

/// Best score for one query word in a title, and the edits it needed. Words
/// of four letters or more may use one adjacent transposition or one
/// substitution when plain subsequence matching fails.
fn match_word(word: &[char], title: &str) -> Option<(i32, u32)> {
    if let Some(score) = subsequence(word, None, title) {
        return Some((score, 0));
    }
    if max_edits(word.len()) == 0 {
        return None;
    }
    let mut best: Option<i32> = None;
    let mut swapped = word.to_vec();
    for i in 0..word.len().saturating_sub(1) {
        swapped.swap(i, i + 1);
        best = best.max(subsequence(&swapped, None, title));
        swapped.swap(i, i + 1);
    }
    for i in 0..word.len() {
        best = best.max(subsequence(word, Some(i), title));
    }
    best.map(|score| (score - TITLE_EDIT_PENALTY, 1))
}

/// Character presence for cheap, safe rejection of impossible ASCII matches.
fn ascii_mask(chars: impl Iterator<Item = char>) -> u128 {
    chars
        .filter(char::is_ascii)
        .fold(0, |mask, c| mask | (1u128 << c as u32))
}

/// Fuzzy score of a whole query against a title: every word must match.
fn fuzzy_score(
    words: &[Vec<char>],
    masks: &[u128],
    repeats: &[Vec<(usize, usize)>],
    edit_ok: &[bool],
    title: &TitleEntry,
) -> Option<i32> {
    let mut total = 0;
    for (((w, mask), repeated), &edits) in words.iter().zip(masks).zip(repeats).zip(edit_ok) {
        // Skipped letters cannot introduce a missing character. A single
        // substitution can supply at most one distinct missing character;
        // transpositions preserve the set. This is a safe rejection only:
        // repeated-letter deficits add safe rejections below. Order and
        // non-ASCII characters still use the full scorer.
        let mut missing = (mask & !title.ascii_mask).count_ones() as usize;
        let substitutions = usize::from(edits && w.len() >= 4);
        for &(letter, count) in repeated {
            let have = title.letters[letter];
            // A saturated count is unknown; leave it to the full scorer.
            if have != u8::MAX {
                missing += count.saturating_sub(usize::from(have)) - usize::from(have == 0);
            }
        }
        if missing > substitutions {
            return None;
        }
        // Words present in the vocabulary must match as typed, in order.
        total += if edits {
            match_word(w, &title.plain)?.0
        } else {
            let typed: String = w.iter().collect();
            if !title.plain.contains(&typed) {
                return None;
            }
            subsequence(w, None, &title.plain)?
        };
    }
    Some(total)
}

/// The title's word that best matches each query word, joined with spaces.
/// When no title word matches (the typed word is longer than any title word
/// it stands for, as in `tomatoe` for `tomato`), the title word with the
/// fewest edits is used. A title without words leaves the word as typed.
fn corrected_title(words: &[Vec<char>], edit_ok: &[bool], title: &TitleEntry) -> String {
    let title_words: Vec<&str> = title
        .plain
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let mut dist = Vec::new();
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    for (word, &edits) in words.iter().zip(edit_ok) {
        // A word present in the vocabulary is never replaced.
        if !edits {
            out.push(word.iter().collect());
            continue;
        }
        let matched = title_words
            .iter()
            .filter_map(|w| match_word(word, w).map(|(score, _)| (score, *w)))
            .max_by_key(|(score, _)| *score)
            .map(|(_, w)| w);
        let chosen = matched.or_else(|| {
            title_words
                .iter()
                .copied()
                .min_by_key(|w| osa(word, &w.chars().collect::<Vec<char>>(), &mut dist))
        });
        out.push(match chosen {
            Some(w) => w.to_string(),
            None => word.iter().collect(),
        });
    }
    out.join(" ")
}

/// Per-query fuzzy inputs: each word's characters, its ASCII letter mask and
/// its repeated letters. Prepared once for each title pass.
#[derive(Default)]
struct FuzzyInputs {
    chars: Vec<Vec<char>>,
    masks: Vec<u128>,
    repeats: Vec<Vec<(usize, usize)>>,
}

impl FuzzyInputs {
    fn new(words: &[String]) -> FuzzyInputs {
        let chars: Vec<Vec<char>> = words.iter().map(|w| w.chars().collect()).collect();
        let masks: Vec<u128> = chars
            .iter()
            .map(|w| ascii_mask(w.iter().copied()))
            .collect();
        let repeats: Vec<Vec<(usize, usize)>> = chars
            .iter()
            .map(|word| {
                let mut counts = [0usize; 26];
                for &c in word.iter().filter(|c| c.is_ascii_lowercase()) {
                    counts[c as usize - 'a' as usize] += 1;
                }
                counts
                    .into_iter()
                    .enumerate()
                    .filter(|(_, count)| *count > 1)
                    .collect()
            })
            .collect();
        FuzzyInputs {
            chars,
            masks,
            repeats,
        }
    }
}

/// Whether a title may be a hit at all: inside the candidate set, not an
/// external row when pages only, and not already returned by an earlier tier.
fn in_scope(
    t: &TitleEntry,
    skip: &HashSet<String>,
    candidates: Option<&HashSet<String>>,
    pages_only: bool,
) -> bool {
    if candidates.is_some_and(|ids| !ids.contains(&t.id)) {
        return false;
    }
    // External identities contain the unit separator; pages never do.
    if pages_only && t.id.contains(crate::index::SEP) {
        return false;
    }
    !skip.contains(&t.id)
}

/// Title hits for `words` (plain, as `query_words` returns them), best first,
/// at most `limit`, leaving out the ids in `skip`.
///
/// Uncorrected tiers (exact, prefix, word-start, substring) come first, in
/// that order, with no score: ties go to the shorter title, then the more
/// recent one, then the id. Only when `allow_fuzzy` is set do titles without
/// an uncorrected match get subsequence or one-edit scoring; those form the
/// last tier, ordered by score, and are the only hits with `fuzzy` set and a
/// `corrected` string (the title's words for each query word).
#[cfg(test)]
pub(crate) fn title_hits(
    titles: &[TitleEntry],
    query: &str,
    words: &[String],
    skip: &HashSet<String>,
    limit: usize,
    allow_fuzzy: bool,
    candidates: Option<&HashSet<String>>,
) -> Vec<SearchHit> {
    title_hits_scoped(
        titles,
        query,
        words,
        skip,
        limit,
        allow_fuzzy,
        candidates,
        false,
    )
}

/// `pages_only` drops external titles before the limit applies.
#[allow(clippy::too_many_arguments)]
pub(crate) fn title_hits_scoped(
    titles: &[TitleEntry],
    query: &str,
    words: &[String],
    skip: &HashSet<String>,
    limit: usize,
    allow_fuzzy: bool,
    candidates: Option<&HashSet<String>>,
    pages_only: bool,
) -> Vec<SearchHit> {
    let all = vec![true; words.len()];
    title_hits_inner(
        titles,
        query,
        words,
        &all,
        skip,
        limit,
        allow_fuzzy,
        false,
        candidates,
        pages_only,
    )
}

/// Called only with spare slots after every uncorrected title was returned.
/// The skipped IDs cover those titles, so classification need not run twice.
/// `missing[i]` says whether word `i` may use edits: only words absent from
/// the vocabulary may, and the others must match as typed.
/// `pages_only` drops external titles before the limit applies.
#[cfg(test)]
pub(crate) fn fuzzy_title_hits_scoped(
    titles: &[TitleEntry],
    words: &[String],
    missing: &[bool],
    skip: &HashSet<String>,
    limit: usize,
    candidates: Option<&HashSet<String>>,
    pages_only: bool,
) -> Vec<SearchHit> {
    title_hits_inner(
        titles, "", words, missing, skip, limit, true, true, candidates, pages_only,
    )
}

/// IDs of the titles `fuzzy_title_hits_scoped` would return for the same
/// inputs, up to `limit`, with no ranking, sorting or correction text. Every
/// title passes the same scope and fuzzy-score test. Scanning stops once
/// `limit` titles qualify. At saturation the kept IDs are the first qualifying
/// ones in `titles` order, not the top-scored ones, so callers may use it
/// only for membership and counts, not for order or the ranked page.
#[cfg(test)]
pub(crate) fn fuzzy_title_ids_scoped(
    titles: &[TitleEntry],
    words: &[String],
    missing: &[bool],
    skip: &HashSet<String>,
    limit: usize,
    candidates: Option<&HashSet<String>>,
    pages_only: bool,
) -> Vec<String> {
    if words.is_empty() || limit == 0 {
        return Vec::new();
    }
    debug_assert_eq!(words.len(), missing.len());
    let inputs = FuzzyInputs::new(words);
    let mut ids = Vec::new();
    for t in titles {
        if !in_scope(t, skip, candidates, pages_only) {
            continue;
        }
        if fuzzy_score(&inputs.chars, &inputs.masks, &inputs.repeats, missing, t).is_some() {
            ids.push(t.id.clone());
            if ids.len() == limit {
                break;
            }
        }
    }
    ids
}

#[allow(clippy::too_many_arguments)]
fn title_hits_inner(
    titles: &[TitleEntry],
    query: &str,
    words: &[String],
    edit_ok: &[bool],
    skip: &HashSet<String>,
    limit: usize,
    allow_fuzzy: bool,
    only_fuzzy: bool,
    candidates: Option<&HashSet<String>>,
    pages_only: bool,
) -> Vec<SearchHit> {
    if words.is_empty() || limit == 0 {
        return Vec::new();
    }
    debug_assert_eq!(words.len(), edit_ok.len());
    let query = plain(query.trim());
    let required_mask = ascii_mask(words.iter().flat_map(|w| w.chars()));
    let inputs = if allow_fuzzy {
        FuzzyInputs::new(words)
    } else {
        FuzzyInputs::default()
    };
    // (tier, fuzzy score (fuzzy tier only), entry)
    let mut found: Vec<(u8, i32, &TitleEntry)> = Vec::new();
    for t in titles {
        if candidates.is_some_and(|ids| !ids.contains(&t.id)) {
            continue;
        }
        if pages_only && t.id.contains(crate::index::SEP) {
            continue;
        }
        // Reject impossible exact matches before checking skipped ids.
        if !allow_fuzzy && required_mask & !t.ascii_mask != 0 {
            continue;
        }
        if skip.contains(&t.id) {
            continue;
        }
        let tier = if only_fuzzy {
            None
        } else {
            uncorrected_tier(&query, words, &t.plain)
        };
        if let Some(tier) = tier {
            found.push((tier, 0, t));
        } else if allow_fuzzy {
            let score = fuzzy_score(&inputs.chars, &inputs.masks, &inputs.repeats, edit_ok, t);
            if let Some(score) = score {
                found.push((FUZZY, score, t));
            }
        }
    }
    found.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| {
                if a.0 == FUZZY {
                    b.1.cmp(&a.1)
                } else {
                    Ordering::Equal
                }
            })
            .then(a.2.chars.cmp(&b.2.chars))
            .then(b.2.updated_ms.cmp(&a.2.updated_ms))
            .then(a.2.id.cmp(&b.2.id))
    });
    found.truncate(limit);
    found
        .into_iter()
        .map(|(tier, _, t)| {
            let fuzzy = tier == FUZZY;
            SearchHit {
                id: t.id.clone(),
                title: t.title.clone(),
                snippet: String::new(),
                fuzzy,
                corrected: fuzzy.then(|| corrected_title(&inputs.chars, edit_ok, t)),
                matched_filters: Vec::new(),
                source: String::new(),
                page_type: String::new(),
                target: None,
            }
        })
        .collect()
}

/// One correction of a body word: the indexed word, its edits from the typed
/// word and how many documents contain it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub term: String,
    pub edits: u32,
    pub docs: u32,
}

/// Reusable buffer for edit distance.
#[derive(Default)]
pub(crate) struct Scratch {
    dist: Vec<u32>,
}

/// Optimal string alignment distance: insertions, deletions, substitutions
/// and adjacent transpositions each cost one.
fn osa(a: &[char], b: &[char], dist: &mut Vec<u32>) -> u32 {
    let w = b.len() + 1;
    dist.clear();
    dist.resize((a.len() + 1) * w, 0);
    for i in 0..=a.len() {
        dist[i * w] = i as u32;
    }
    for (j, value) in dist.iter_mut().take(w).enumerate() {
        *value = j as u32;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = u32::from(a[i - 1] != b[j - 1]);
            let mut v = (dist[(i - 1) * w + j] + 1)
                .min(dist[i * w + j - 1] + 1)
                .min(dist[(i - 1) * w + j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(dist[(i - 2) * w + j - 2] + 1);
            }
            dist[i * w + j] = v;
        }
    }
    dist[a.len() * w + b.len()]
}

/// FNV-1a over the characters of `chars` except positions `i` and `j`.
fn hash_without(chars: &[char], i: usize, j: usize) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for (k, c) in chars.iter().enumerate() {
        if k != i && k != j {
            h ^= *c as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

/// Hashes of `chars` and of each form with one or two characters deleted.
/// Two words within two edits share one of these forms (SymSpell).
fn deletion_hashes(chars: &[char], out: &mut Vec<u64>) {
    let none = usize::MAX;
    out.clear();
    out.push(hash_without(chars, none, none));
    for i in 0..chars.len() {
        out.push(hash_without(chars, i, none));
        for j in i + 1..chars.len() {
            out.push(hash_without(chars, i, j));
        }
    }
}

/// The vocabulary and its indexes. `words` (distinct FTS terms with document
/// counts) is sorted by text, so `chars` (the same terms as characters) is in
/// the same order. `keys` holds sorted (hash, word number) pairs for each
/// word's one- and two-deletion forms, serving whole-word correction. Prefix
/// correction walks the sorted `chars` as an implicit trie instead.
#[derive(Default)]
pub(crate) struct WordIndex {
    pub words: Vec<(String, u32)>,
    chars: Vec<Vec<char>>,
    keys: Vec<(u64, u32)>,
}

impl WordIndex {
    pub fn build(mut words: Vec<(String, u32)>) -> WordIndex {
        words.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        words.shrink_to_fit();
        let chars: Vec<Vec<char>> = words
            .iter()
            .map(|(t, _)| {
                let mut c: Vec<char> = t.chars().collect();
                c.shrink_to_fit();
                c
            })
            .collect();
        let mut keys = Vec::new();
        let mut hashes = Vec::new();
        for (n, chs) in chars.iter().enumerate() {
            if chs.len() < 2 {
                continue;
            }
            deletion_hashes(chs, &mut hashes);
            keys.extend(hashes.iter().map(|h| (*h, n as u32)));
        }
        keys.sort_unstable();
        keys.dedup();
        keys.shrink_to_fit();
        WordIndex { words, chars, keys }
    }

    pub fn key_count(&self) -> usize {
        self.keys.len()
    }

    /// Owned heap retained by the word index, by allocated capacity: the
    /// words and character vectors (both outer vectors and their contents)
    /// and the key vector. Not counted: allocator block overhead.
    pub fn estimated_bytes(&self) -> u64 {
        let word_vec = self.words.capacity() * std::mem::size_of::<(String, u32)>();
        let word_text: usize = self.words.iter().map(|(t, _)| t.capacity()).sum();
        let char_vec = self.chars.capacity() * std::mem::size_of::<Vec<char>>();
        let char_text: usize = self
            .chars
            .iter()
            .map(|c| c.capacity() * std::mem::size_of::<char>())
            .sum();
        let keys = self.keys.capacity() * std::mem::size_of::<(u64, u32)>();
        (word_vec + word_text + char_vec + char_text + keys) as u64
    }

    /// Indexed words within `max_edits` of a whole (not last) query word,
    /// most frequent first. Words of up to three letters are never corrected.
    pub fn correct(&self, word: &[char], s: &mut Scratch) -> Vec<Candidate> {
        let max = max_edits(word.len());
        if max == 0 {
            return Vec::new();
        }
        let mut hashes = Vec::new();
        deletion_hashes(word, &mut hashes);
        let mut ids = Vec::new();
        for h in hashes {
            let start = self.keys.partition_point(|k| k.0 < h);
            ids.extend(
                self.keys[start..]
                    .iter()
                    .take_while(|k| k.0 == h)
                    .map(|k| k.1),
            );
        }
        ids.sort_unstable();
        ids.dedup();
        let mut out = Vec::new();
        for id in ids {
            let n = id as usize;
            let edits = osa(word, &self.chars[n], &mut s.dist);
            if edits >= 1 && edits <= max {
                out.push(Candidate {
                    term: self.words[n].0.clone(),
                    edits,
                    docs: self.words[n].1,
                });
            }
        }
        rank(out)
    }

    /// Actual indexed words whose start is within `max_edits` of the last
    /// query word, which is still being typed. Each word appears once, with
    /// its fewest edits against any of its prefixes; the word itself is
    /// returned, never a truncated prefix. Words that start with the typed
    /// text are exact and left out.
    ///
    /// The vocabulary is walked as an implicit trie (sorted ranges, one
    /// character per level) with the rolling edit-distance rows, pruned as
    /// soon as no extension can be within the limit. Only the subtrees that
    /// survive are visited, not the whole vocabulary.
    pub fn correct_prefix(&self, word: &[char], _scratch: &mut Scratch) -> Vec<Candidate> {
        let max = max_edits(word.len());
        if max == 0 || self.chars.is_empty() {
            return Vec::new();
        }
        let first: Vec<u32> = (0..=word.len() as u32).collect();
        let mut found: HashMap<usize, u32> = HashMap::new();
        self.walk(word, max, 0, 0..self.chars.len(), None, &first, &mut found);
        let out: Vec<Candidate> = found
            .into_iter()
            .filter(|(n, _)| !self.chars[*n].starts_with(word))
            .map(|(n, edits)| Candidate {
                term: self.words[n].0.clone(),
                edits,
                docs: self.words[n].1,
            })
            .collect();
        rank(out)
    }

    /// One trie level. `range` holds the words sharing the path so far (length
    /// `depth`); `row` is the edit-distance row for that path against the
    /// whole query; `prev` is the row one level up and the path's last
    /// character, for transpositions. Each child whose path is within the
    /// limit records the distance for every word under it (keeping the least).
    #[allow(clippy::too_many_arguments)]
    fn walk(
        &self,
        q: &[char],
        max: u32,
        depth: usize,
        range: Range<usize>,
        prev: Option<(&[u32], char)>,
        row: &[u32],
        found: &mut HashMap<usize, u32>,
    ) {
        let len = q.len();
        if depth >= len + max as usize {
            return;
        }
        let mut i = range.start;
        while i < range.end {
            // Words that end here sort first in the range; they have no child.
            if self.chars[i].len() == depth {
                i += 1;
                continue;
            }
            let c = self.chars[i][depth];
            let end =
                i + self.chars[i..range.end].partition_point(|w| w.len() > depth && w[depth] <= c);
            let mut next = vec![0u32; len + 1];
            next[0] = depth as u32 + 1;
            for j in 1..=len {
                let cost = u32::from(q[j - 1] != c);
                let mut v = (row[j] + 1).min(next[j - 1] + 1).min(row[j - 1] + cost);
                if let Some((prev_row, last)) = prev {
                    if j >= 2 && c == q[j - 2] && last == q[j - 1] {
                        v = v.min(prev_row[j - 2] + 1);
                    }
                }
                next[j] = v;
            }
            let delta = next[len];
            if delta == 0 {
                // The typed text is a prefix of every word here: exact.
                i = end;
                continue;
            }
            if delta <= max {
                for n in i..end {
                    found
                        .entry(n)
                        .and_modify(|e| *e = (*e).min(delta))
                        .or_insert(delta);
                }
            }
            // Row minima never fall as the path grows, so a row whose best
            // cell is already past the limit cannot recover below it.
            let next_min = next.iter().copied().min().unwrap_or(0);
            if next_min <= max {
                self.walk(q, max, depth + 1, i..end, Some((row, c)), &next, found);
            }
            i = end;
        }
    }
}

/// Most frequent first, then fewest edits, then longer terms (a whole word
/// over a shorter one), then the term; at most `MAX_CANDIDATES`.
fn rank(mut out: Vec<Candidate>) -> Vec<Candidate> {
    out.sort_by(|a, b| {
        b.docs
            .cmp(&a.docs)
            .then(a.edits.cmp(&b.edits))
            .then(b.term.chars().count().cmp(&a.term.chars().count()))
            .then(a.term.cmp(&b.term))
    });
    out.truncate(MAX_CANDIDATES);
    out
}

/// Cached titles and word index, rebuilt when the database has changed since
/// they were built (`index.rs`, `refresh_fuzzy`).
#[derive(Default)]
pub(crate) struct Cache {
    /// (data_version, total_changes) at build time.
    pub guard: Option<(i64, u64)>,
    pub titles: Vec<TitleEntry>,
    pub words: WordIndex,
    pub build_ms: f64,
    /// Scores of the last title query. Cleared by `clear_title_matches`.
    title_scores: Option<TitleScores>,
}

/// Fuzzy scores of `Cache::titles` for one query. The scores depend only on
/// the title vector and the query, so skips, candidate sets and page-only
/// filters are applied when hits or ids are read, not stored here.
struct TitleScores {
    words: Vec<String>,
    missing: Vec<bool>,
    titles_len: usize,
    inputs: FuzzyInputs,
    /// Qualifying titles as (index into the title vector, score), in title order.
    scored: Vec<(usize, i32)>,
    /// `scored` in fuzzy hit order, built on the first hit request.
    ranked: Option<Vec<(usize, i32)>>,
}

impl TitleScores {
    fn new(titles: &[TitleEntry], words: &[String], missing: &[bool]) -> TitleScores {
        debug_assert_eq!(words.len(), missing.len());
        let inputs = FuzzyInputs::new(words);
        let scored = titles
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                let score = fuzzy_score(&inputs.chars, &inputs.masks, &inputs.repeats, missing, t)?;
                Some((i, score))
            })
            .collect();
        TitleScores {
            words: words.to_vec(),
            missing: missing.to_vec(),
            titles_len: titles.len(),
            inputs,
            scored,
            ranked: None,
        }
    }

    fn matches(&self, titles_len: usize, words: &[String], missing: &[bool]) -> bool {
        self.titles_len == titles_len && self.words == words && self.missing == missing
    }
}

/// The scores for this query, rebuilt unless the last query had the same
/// words, edit flags and title count.
fn title_scores_for<'a>(
    slot: &'a mut Option<TitleScores>,
    titles: &[TitleEntry],
    words: &[String],
    missing: &[bool],
) -> &'a mut TitleScores {
    if !slot
        .as_ref()
        .is_some_and(|s| s.matches(titles.len(), words, missing))
    {
        *slot = None;
    }
    slot.get_or_insert_with(|| TitleScores::new(titles, words, missing))
}

/// The order `title_hits_inner` gives fuzzy hits: score, then shorter title,
/// then more recent, then id.
fn fuzzy_order(a: &TitleEntry, a_score: i32, b: &TitleEntry, b_score: i32) -> Ordering {
    b_score
        .cmp(&a_score)
        .then(a.chars.cmp(&b.chars))
        .then(b.updated_ms.cmp(&a.updated_ms))
        .then(a.id.cmp(&b.id))
}

/// The hit `title_hits_inner` builds for a fuzzy-tier title.
fn fuzzy_hit(t: &TitleEntry, words: &[Vec<char>], edit_ok: &[bool]) -> SearchHit {
    SearchHit {
        id: t.id.clone(),
        title: t.title.clone(),
        snippet: String::new(),
        fuzzy: true,
        corrected: Some(corrected_title(words, edit_ok, t)),
        matched_filters: Vec::new(),
        source: String::new(),
        page_type: String::new(),
        target: None,
    }
}

impl Cache {
    /// Drops the scores of the last title query. Call whenever `titles` is
    /// replaced or patched: the scores index into that vector.
    pub(crate) fn clear_title_matches(&mut self) {
        self.title_scores = None;
    }

    /// `fuzzy_title_hits_scoped` over `self.titles`, reusing the scores of the
    /// last query. Hits, their order, labels and limits are the same.
    pub(crate) fn fuzzy_title_hits_scoped(
        &mut self,
        words: &[String],
        missing: &[bool],
        skip: &HashSet<String>,
        limit: usize,
        candidates: Option<&HashSet<String>>,
        pages_only: bool,
    ) -> Vec<SearchHit> {
        if words.is_empty() || limit == 0 {
            return Vec::new();
        }
        let titles = self.titles.as_slice();
        let scores = title_scores_for(&mut self.title_scores, titles, words, missing);
        if scores.ranked.is_none() {
            let mut ranked = scores.scored.clone();
            ranked.sort_by(|a, b| fuzzy_order(&titles[a.0], a.1, &titles[b.0], b.1));
            scores.ranked = Some(ranked);
        }
        let ranked = scores.ranked.as_deref().unwrap_or(&[]);
        ranked
            .iter()
            .map(|&(i, _)| &titles[i])
            .filter(|t| in_scope(t, skip, candidates, pages_only))
            .take(limit)
            .map(|t| fuzzy_hit(t, &scores.inputs.chars, missing))
            .collect()
    }

    /// `fuzzy_title_ids_scoped` over `self.titles`, reusing the scores of the
    /// last query. Returns the same IDs as the standalone function.
    pub(crate) fn fuzzy_title_ids_scoped(
        &mut self,
        words: &[String],
        missing: &[bool],
        skip: &HashSet<String>,
        limit: usize,
        candidates: Option<&HashSet<String>>,
        pages_only: bool,
    ) -> Vec<String> {
        if words.is_empty() || limit == 0 {
            return Vec::new();
        }
        let titles = self.titles.as_slice();
        let scores = title_scores_for(&mut self.title_scores, titles, words, missing);
        scores
            .scored
            .iter()
            .map(|&(i, _)| &titles[i])
            .filter(|t| in_scope(t, skip, candidates, pages_only))
            .take(limit)
            .map(|t| t.id.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    fn entry(id: &str, title: &str, updated: i64) -> TitleEntry {
        TitleEntry::new(id.into(), title.into(), updated)
    }

    fn ids(hits: &[SearchHit]) -> Vec<&str> {
        hits.iter().map(|h| h.id.as_str()).collect()
    }

    #[test]
    fn edit_distance_counts_transpositions_as_one() {
        let mut d = Vec::new();
        assert_eq!(osa(&chars("projcet"), &chars("project"), &mut d), 1);
        assert_eq!(osa(&chars("gardne"), &chars("garden"), &mut d), 1);
        assert_eq!(osa(&chars("tomatoe"), &chars("tomato"), &mut d), 1);
        assert_eq!(osa(&chars("cofee"), &chars("coffee"), &mut d), 1);
        assert_eq!(osa(&chars("abc"), &chars("abc"), &mut d), 0);
    }

    #[test]
    fn short_words_never_take_substitutions() {
        assert!(match_word(&chars("cof"), "coffee").is_some());
        assert!(match_word(&chars("cof"), "cfo").is_none());
        assert!(match_word(&chars("pln"), "garden plan").is_some());
    }

    #[test]
    fn length_bands_follow_the_spec() {
        assert_eq!(max_edits(3), 0);
        assert_eq!(max_edits(4), 1);
        assert_eq!(max_edits(7), 1);
        assert_eq!(max_edits(8), 2);
    }

    #[test]
    fn title_tiers_are_uncorrected_and_ordered() {
        let titles = vec![
            entry("5", "Lastweek review", 1),
            entry("3", "Weekly review", 1),
            entry("2", "Week review notes", 1),
            entry("1", "Week rev", 1),
            entry("7", "Wekk review", 1),
        ];
        let words = query_words("week rev");
        let none = HashSet::new();
        // Exact, prefix, word-start, substring: uncorrected and unscored.
        let plain = title_hits(&titles, "week rev", &words, &none, 10, false, None);
        assert_eq!(ids(&plain), vec!["1", "2", "3", "5"]);
        assert!(plain.iter().all(|h| !h.fuzzy && h.corrected.is_none()));
        // The fuzzy tier comes last, labelled and corrected.
        let all = title_hits(&titles, "week rev", &words, &none, 10, true, None);
        assert_eq!(ids(&all), vec!["1", "2", "3", "5", "7"]);
        assert!(all[..4].iter().all(|h| !h.fuzzy && h.corrected.is_none()));
        assert!(all[4].fuzzy);
        assert_eq!(all[4].corrected.as_deref(), Some("wekk review"));
    }

    #[test]
    fn fuzzy_title_correction_falls_back_to_the_nearest_word() {
        let titles = vec![entry("t", "Tomato sauce", 1)];
        let hits = title_hits(
            &titles,
            "tomatoe",
            &query_words("tomatoe"),
            &HashSet::new(),
            10,
            true,
            None,
        );
        assert_eq!(ids(&hits), vec!["t"]);
        assert!(hits[0].fuzzy);
        assert_eq!(hits[0].corrected.as_deref(), Some("tomato"));
        // Without fuzzy matching the same title is not a hit at all.
        assert!(title_hits(
            &titles,
            "tomatoe",
            &query_words("tomatoe"),
            &HashSet::new(),
            10,
            false,
            None
        )
        .is_empty());
    }

    #[test]
    fn ties_prefer_shorter_title_then_recency_then_id() {
        let titles = vec![
            entry("a", "Gardening", 1),
            entry("b", "Garden kit", 100),
            entry("c", "Gardens", 1),
            entry("x", "Garden 3", 50),
            entry("y", "Garden 2", 50),
            entry("z", "Garden 1", 1),
        ];
        let words = query_words("garden");
        let hits = title_hits(&titles, "garden", &words, &HashSet::new(), 10, false, None);
        // Shorter first (7, 8, 9, 10 characters); within 8, recency then id.
        assert_eq!(ids(&hits), vec!["c", "x", "y", "z", "a", "b"]);
    }

    #[test]
    fn skipped_and_limited_titles_are_left_out() {
        let titles = vec![entry("a", "Garden", 1), entry("b", "Garden plan", 1)];
        let words = query_words("garden");
        let skip: HashSet<String> = ["a".to_string()].into_iter().collect();
        assert_eq!(
            ids(&title_hits(
                &titles, "garden", &words, &skip, 10, false, None
            )),
            vec!["b"]
        );
        assert_eq!(
            title_hits(&titles, "garden", &words, &HashSet::new(), 1, false, None).len(),
            1
        );
    }

    #[test]
    fn prefix_correction_returns_whole_words_not_prefixes() {
        let index = WordIndex::build(vec![
            ("coffee".into(), 5),
            ("coffin".into(), 1),
            ("cook".into(), 3),
        ]);
        let mut s = Scratch::default();
        let found = index.correct_prefix(&chars("cofee"), &mut s);
        let terms: Vec<&str> = found.iter().map(|c| c.term.as_str()).collect();
        assert_eq!(terms, vec!["coffee"]);
        assert_eq!(found[0].edits, 1);
        // "coffee" starts with the typed "coffe", so it is exact and left out;
        // "coffin" is one substitution from "coffe" and is the correction.
        let found = index.correct_prefix(&chars("coffe"), &mut s);
        let terms: Vec<&str> = found.iter().map(|c| c.term.as_str()).collect();
        assert_eq!(terms, vec!["coffin"]);
    }

    #[test]
    fn prefix_candidates_are_distinct_and_frequency_ordered() {
        let index = WordIndex::build(vec![
            ("garden".into(), 3),
            ("gardens".into(), 1),
            ("gardener".into(), 2),
            ("gardner".into(), 4),
        ]);
        let mut s = Scratch::default();
        let found = index.correct_prefix(&chars("gardne"), &mut s);
        let terms: Vec<&str> = found.iter().map(|c| c.term.as_str()).collect();
        // "gardner" starts with the typed text (exact) and is left out.
        assert_eq!(terms, vec!["garden", "gardener", "gardens"]);
        assert!(found.iter().all(|c| c.edits == 1));
    }

    #[test]
    fn deletion_index_finds_two_edit_words() {
        let index = WordIndex::build(vec![
            ("recipes".into(), 4),
            ("recipe".into(), 9),
            ("recipies".into(), 1),
        ]);
        let mut s = Scratch::default();
        let found = index.correct(&chars("recipies"), &mut s);
        assert!(found.iter().any(|c| c.term == "recipes" && c.edits == 1));
        // Two substitutions in a nine-letter word: both sides delete the
        // same two positions, so the deletion index finds it.
        let index = WordIndex::build(vec![("gardening".into(), 3)]);
        let found = index.correct(&chars("gardenlnq"), &mut s);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].edits, 2);
        assert!(index.correct(&chars("tea"), &mut s).is_empty());
    }

    /// Titles in iteration order. The external row comes first so that
    /// `pages_only` has to drop it before the limit applies.
    fn scope_fixture() -> Vec<TitleEntry> {
        let external = format!("agent{}chat-1", crate::index::SEP);
        vec![
            entry(&external, "garden chat", 9),
            entry("p1", "garden plan", 1),
            entry("p2", "garden plans", 2),
            entry("p3", "gardens", 3),
            entry("p4", "kitchen garden", 4),
            entry("p5", "budget", 5),
        ]
    }

    fn strings(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    /// Runs both helpers on the same inputs. Returns the new helper's IDs and
    /// the IDs of the existing helper's hits, in that helper's order.
    fn run(
        titles: &[TitleEntry],
        words: &[&str],
        missing: &[bool],
        skip: &HashSet<String>,
        limit: usize,
        candidates: Option<&HashSet<String>>,
        pages_only: bool,
    ) -> (Vec<String>, Vec<String>) {
        let words: Vec<String> = words.iter().map(|w| w.to_string()).collect();
        let helper_ids =
            fuzzy_title_ids_scoped(titles, &words, missing, skip, limit, candidates, pages_only);
        let hits =
            fuzzy_title_hits_scoped(titles, &words, missing, skip, limit, candidates, pages_only);
        (helper_ids, hits.into_iter().map(|h| h.id).collect())
    }

    fn sorted(mut v: Vec<String>) -> Vec<String> {
        v.sort();
        v
    }

    #[test]
    fn unsaturated_ids_match_the_fuzzy_hits() {
        let titles = scope_fixture();
        let none = HashSet::new();
        let (got, want) = run(&titles, &["gardne"], &[true], &none, 50, None, false);
        assert!(!want.is_empty());
        assert_eq!(sorted(got.clone()), sorted(want));
        assert!(got.contains(&"p1".to_string()));
        assert!(
            !got.contains(&"p5".to_string()),
            "budget does not match gardne"
        );
    }

    #[test]
    fn saturated_limits_return_the_bounded_count_of_qualifying_ids() {
        let titles = scope_fixture();
        let none = HashSet::new();
        let (_, all) = run(&titles, &["gardne"], &[true], &none, 50, None, false);
        let total = all.len();
        assert!(
            total >= 3,
            "fixture should give several fuzzy hits: {all:?}"
        );
        for limit in 1..=total + 1 {
            let (got, hits) = run(&titles, &["gardne"], &[true], &none, limit, None, false);
            assert_eq!(got.len(), limit.min(total), "limit {limit}");
            assert_eq!(hits.len(), limit.min(total), "limit {limit}");
            assert!(
                got.iter().all(|id| all.contains(id)),
                "limit {limit}: {got:?}"
            );
            let unique: HashSet<&String> = got.iter().collect();
            assert_eq!(unique.len(), got.len(), "limit {limit}: duplicate ids");
        }
    }

    #[test]
    fn skipped_titles_are_left_out_of_both_helpers() {
        let titles = scope_fixture();
        let skip = strings(&["p1", "p2"]);
        let (got, want) = run(&titles, &["gardne"], &[true], &skip, 50, None, false);
        assert_eq!(sorted(got.clone()), sorted(want));
        assert!(!got.iter().any(|id| skip.contains(id)), "{got:?}");
        assert!(!got.is_empty());
    }

    #[test]
    fn pages_only_drops_external_rows_before_the_limit() {
        let titles = scope_fixture();
        let none = HashSet::new();
        let external = format!("agent{}chat-1", crate::index::SEP);

        let (got, want) = run(&titles, &["gardne"], &[true], &none, 50, None, true);
        assert_eq!(sorted(got.clone()), sorted(want));
        assert!(!got.is_empty());
        assert!(
            !got.iter().any(|id| id.contains(crate::index::SEP)),
            "{got:?}"
        );

        // The external row must not consume the single permitted slot.
        let (got, want) = run(&titles, &["gardne"], &[true], &none, 1, None, true);
        assert_eq!(got.len(), 1);
        assert!(!got[0].contains(crate::index::SEP));
        assert_eq!(want.len(), 1);
        assert!(!want[0].contains(crate::index::SEP));

        let (got, _) = run(&titles, &["gardne"], &[true], &none, 50, None, false);
        assert!(got.contains(&external));
    }

    #[test]
    fn hard_candidate_sets_limit_titles_before_the_limit() {
        let titles = scope_fixture();
        let none = HashSet::new();

        let only_budget = strings(&["p5"]);
        let (got, want) = run(
            &titles,
            &["gardne"],
            &[true],
            &none,
            50,
            Some(&only_budget),
            false,
        );
        assert!(got.is_empty());
        assert!(want.is_empty());

        let only_kitchen = strings(&["p4"]);
        let (got, want) = run(
            &titles,
            &["gardne"],
            &[true],
            &none,
            50,
            Some(&only_kitchen),
            false,
        );
        assert_eq!(got, vec!["p4".to_string()]);
        assert_eq!(want, got);

        let some = strings(&["p2", "p3", "p5"]);
        let (got, want) = run(&titles, &["gardne"], &[true], &none, 50, Some(&some), false);
        assert_eq!(sorted(got.clone()), sorted(want));
        assert!(got.iter().all(|id| some.contains(id)), "{got:?}");
    }

    #[test]
    fn words_that_may_not_use_edits_must_match_as_typed() {
        let titles = scope_fixture();
        let none = HashSet::new();

        // "gardne" is not a substring of any title, so it cannot match exactly.
        let (got, want) = run(&titles, &["gardne"], &[false], &none, 50, None, false);
        assert!(got.is_empty());
        assert!(want.is_empty());

        // Only the edit-eligible word may be fuzzy; "plan" must appear as typed.
        let (got, want) = run(
            &titles,
            &["gardne", "plan"],
            &[true, false],
            &none,
            50,
            None,
            false,
        );
        assert_eq!(
            sorted(got.clone()),
            vec!["p1".to_string(), "p2".to_string()]
        );
        assert_eq!(sorted(want), sorted(got));

        let words = ["garden", "plan"];
        let (got, want) = run(&titles, &words, &[false, false], &none, 50, None, false);
        assert_eq!(
            sorted(got.clone()),
            vec!["p1".to_string(), "p2".to_string()]
        );
        assert_eq!(sorted(want), sorted(got));
    }

    #[test]
    fn empty_words_and_zero_limit_return_nothing() {
        let titles = scope_fixture();
        let none = HashSet::new();
        let (got, want) = run(&titles, &[], &[], &none, 5, None, false);
        assert!(got.is_empty() && want.is_empty());
        let (got, want) = run(&titles, &["gardne"], &[true], &none, 0, None, false);
        assert!(got.is_empty() && want.is_empty());
    }

    /// Hit identity for comparison: id, fuzzy flag and correction label.
    fn shape(hits: &[SearchHit]) -> Vec<(String, bool, Option<String>)> {
        hits.iter()
            .map(|h| (h.id.clone(), h.fuzzy, h.corrected.clone()))
            .collect()
    }

    /// Checks both cached methods against the standalone oracles over the
    /// cache's own titles.
    fn check_cached(
        cache: &mut Cache,
        words: &[&str],
        missing: &[bool],
        skip: &HashSet<String>,
        limit: usize,
        candidates: Option<&HashSet<String>>,
        pages_only: bool,
    ) {
        let words: Vec<String> = words.iter().map(|w| w.to_string()).collect();
        let label = format!("{words:?} {missing:?} limit {limit} pages_only {pages_only}");

        let scope = (skip, limit, candidates, pages_only);
        let want = fuzzy_title_hits_scoped(
            &cache.titles,
            &words,
            missing,
            scope.0,
            scope.1,
            scope.2,
            scope.3,
        );
        let got =
            cache.fuzzy_title_hits_scoped(&words, missing, scope.0, scope.1, scope.2, scope.3);
        assert_eq!(shape(&got), shape(&want), "hits: {label}");

        let want_ids = fuzzy_title_ids_scoped(
            &cache.titles,
            &words,
            missing,
            scope.0,
            scope.1,
            scope.2,
            scope.3,
        );
        let got_ids =
            cache.fuzzy_title_ids_scoped(&words, missing, scope.0, scope.1, scope.2, scope.3);
        assert_eq!(sorted(got_ids), sorted(want_ids), "ids: {label}");
    }

    #[test]
    fn cached_methods_match_standalone_across_queries_scopes_and_limits() {
        let mut cache = Cache {
            titles: scope_fixture(),
            ..Default::default()
        };
        // Alternating queries also exercise replacement of the one cached query.
        let queries: &[(&[&str], &[bool])] = &[
            (&["gardne"], &[true]),
            (&["gardne", "plan"], &[true, false]),
            (&["garden", "plan"], &[false, false]),
            (&["plan"], &[false]),
            (&["gardne"], &[false]),
        ];
        let skips = [HashSet::new(), strings(&["p1"]), strings(&["p1", "p4"])];
        let candidate_sets = [None, Some(strings(&["p2", "p3", "p4", "p5"]))];
        for (words, missing) in queries {
            for skip in &skips {
                for candidates in &candidate_sets {
                    for pages_only in [false, true] {
                        for limit in 0..=7 {
                            check_cached(
                                &mut cache,
                                words,
                                missing,
                                skip,
                                limit,
                                candidates.as_ref(),
                                pages_only,
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn cached_hits_break_ties_like_the_standalone_sort() {
        // Same title text, so the same score and length: recency, then id.
        let mut cache = Cache {
            titles: vec![
                entry("c", "garden plan", 2),
                entry("b", "garden plan", 1),
                entry("a", "garden plan", 1),
            ],
            ..Default::default()
        };
        let none = HashSet::new();
        check_cached(&mut cache, &["gardne"], &[true], &none, 10, None, false);
        let words = ["gardne".to_string()];
        let got = cache.fuzzy_title_hits_scoped(&words, &[true], &none, 10, None, false);
        let order: Vec<&str> = got.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(order, ["c", "a", "b"]);
    }

    #[test]
    fn clearing_after_title_changes_reflects_the_new_vector() {
        let mut cache = Cache {
            titles: scope_fixture(),
            ..Default::default()
        };
        let none = HashSet::new();
        check_cached(&mut cache, &["gardne"], &[true], &none, 50, None, false);

        // A grown vector rebuilds the scores even without a clear.
        cache.titles.push(entry("p6", "garden shed", 6));
        check_cached(&mut cache, &["gardne"], &[true], &none, 50, None, false);

        // Dropping a row shifts the indices, so the caller must clear.
        cache.titles.retain(|t| t.id != "p4");
        cache.clear_title_matches();
        check_cached(&mut cache, &["gardne"], &[true], &none, 50, None, false);
        let words = ["gardne".to_string()];
        let ids = cache.fuzzy_title_ids_scoped(&words, &[true], &none, 50, None, false);
        assert!(ids.contains(&"p6".to_string()));
        assert!(!ids.contains(&"p4".to_string()));
    }

    #[test]
    fn empty_words_and_zero_limit_return_nothing_from_the_cache() {
        let mut cache = Cache {
            titles: scope_fixture(),
            ..Default::default()
        };
        let none = HashSet::new();
        check_cached(&mut cache, &[], &[], &none, 5, None, false);
        check_cached(&mut cache, &["gardne"], &[true], &none, 0, None, false);
        assert!(cache
            .fuzzy_title_hits_scoped(&[], &[], &none, 5, None, false)
            .is_empty());
    }
}
