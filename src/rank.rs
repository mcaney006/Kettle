//! Ranking and ordering primitives.
//!
//! Two things live here, both hot and both easy to get subtly wrong:
//!   * `score`       -- fuzzy match scoring for the search box (16k candidates/keystroke)
//!   * `version_cmp` -- Homebrew-compatible version ordering
//!
//! Both are pure functions over bytes/str so they are cheap to test exhaustively.

use std::cmp::Ordering;

// ---- fuzzy matching ---------------------------------------------------------

// Weights follow fzf's model: matching is cheap, *tight and boundary-aligned*
// matching is what actually signals intent.
const SCORE_MATCH: i32 = 16;
const BONUS_BOUNDARY: i32 = 8;
const BONUS_CAMEL: i32 = 7;
const BONUS_CONSECUTIVE: i32 = 8;
const BONUS_FIRST_CHAR: i32 = 12;
const PENALTY_GAP_START: i32 = -3;
const PENALTY_GAP_EXTEND: i32 = -1;

#[inline]
fn is_sep(b: u8) -> bool {
    matches!(b, b'-' | b'_' | b'.' | b'/' | b'@' | b' ' | b'+')
}

/// Bonus for starting a match at `i`, judged from the *preceding* byte.
#[inline]
fn boundary_bonus(hay: &[u8], i: usize) -> i32 {
    if i == 0 {
        return BONUS_BOUNDARY + BONUS_FIRST_CHAR;
    }
    let prev = hay[i - 1];
    if is_sep(prev) {
        BONUS_BOUNDARY
    } else if prev.is_ascii_lowercase() && hay[i].is_ascii_uppercase() {
        BONUS_CAMEL
    } else {
        0
    }
}

/// Score `needle` against `hay`. Both must already be lowercased ASCII-folded
/// (see `fold`), except `hay_raw` which retains case for camelCase bonuses.
///
/// Returns `None` if `needle` is not a subsequence of `hay`.
///
/// Strategy (fzf's): a forward greedy pass finds the earliest index at which the
/// needle is fully consumed -- that is the end of the *first* feasible match. A
/// backward greedy pass from that end finds the latest feasible start, giving the
/// minimal window containing a match. Aligning inside that window is what makes
/// "rip" rank `ripgrep` (tight, consecutive) above `rust-in-peace` (scattered),
/// which naively tightening each character independently gets backwards.
pub fn score(needle: &[u8], hay: &[u8], hay_raw: &[u8]) -> Option<i32> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > hay.len() {
        return None;
    }
    let n = needle.len().min(64);

    // Forward: earliest end of a feasible match.
    let mut k = 0usize;
    let mut end = None;
    for (i, &hb) in hay.iter().enumerate() {
        if hb == needle[k] {
            k += 1;
            if k == n {
                end = Some(i);
                break;
            }
        }
    }
    let end = end?;

    // Backward from that end: latest feasible start.
    let mut k = n;
    let mut start = 0usize;
    for i in (0..=end).rev() {
        if hay[i] == needle[k - 1] {
            k -= 1;
            if k == 0 {
                start = i;
                break;
            }
        }
    }

    // Align greedily inside the minimal window.
    let mut pos = [0usize; 64];
    let mut k = 0usize;
    for i in start..=end {
        if k < n && hay[i] == needle[k] {
            pos[k] = i;
            k += 1;
        }
    }

    // Score the alignment.
    let mut total = 0i32;
    let mut prev: Option<usize> = None;
    let mut in_gap = false;
    for k in 0..n {
        let i = pos[k];
        total += SCORE_MATCH;
        match prev {
            Some(p) if i == p + 1 => {
                total += BONUS_CONSECUTIVE;
                in_gap = false;
            }
            Some(_) => {
                total += if in_gap { PENALTY_GAP_EXTEND } else { PENALTY_GAP_START };
                in_gap = true;
                total += boundary_bonus(hay_raw, i);
            }
            None => total += boundary_bonus(hay_raw, i),
        }
        prev = Some(i);
    }

    // Prefer shorter haystacks and matches that finish early.
    total -= (hay.len() as i32) / 8;
    Some(total)
}

/// ASCII-lowercase fold used for both needle and haystack.
pub fn fold(s: &str) -> Vec<u8> {
    s.bytes().map(|b| b.to_ascii_lowercase()).collect()
}

// ---- version ordering -------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum Tok {
    /// Pre-release marker; lower rank sorts earlier (alpha < beta < pre < rc).
    Pre(u8),
    Num(u64),
    Str(String),
}

fn pre_rank(s: &str) -> Option<u8> {
    match s {
        "alpha" | "a" => Some(0),
        "beta" | "b" => Some(1),
        "pre" => Some(2),
        "rc" => Some(3),
        _ => None,
    }
}

/// Split a Homebrew version into comparable tokens plus its `_N` revision.
fn tokenize(v: &str) -> (Vec<Tok>, u64) {
    // Homebrew appends `_N` for a rebuild of the same upstream version.
    let (base, rev) = match v.rsplit_once('_') {
        Some((b, r)) if !r.is_empty() && r.bytes().all(|c| c.is_ascii_digit()) => {
            (b, r.parse().unwrap_or(0))
        }
        _ => (v, 0),
    };

    let mut toks = Vec::new();
    let bytes = base.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if !c.is_ascii_alphanumeric() {
            i += 1;
            continue;
        }
        let start = i;
        let numeric = c.is_ascii_digit();
        while i < bytes.len()
            && bytes[i].is_ascii_alphanumeric()
            && bytes[i].is_ascii_digit() == numeric
        {
            i += 1;
        }
        let s = &base[start..i];
        toks.push(if numeric {
            // Saturate rather than wrap on absurd version components.
            Tok::Num(s.parse().unwrap_or(u64::MAX))
        } else {
            let l = s.to_ascii_lowercase();
            match pre_rank(&l) {
                Some(r) => Tok::Pre(r),
                None => Tok::Str(l),
            }
        });
    }
    (toks, rev)
}

fn cmp_tok(a: &Tok, b: &Tok) -> Ordering {
    match (a, b) {
        (Tok::Num(x), Tok::Num(y)) => x.cmp(y),
        (Tok::Str(x), Tok::Str(y)) => x.cmp(y),
        (Tok::Pre(x), Tok::Pre(y)) => x.cmp(y),
        // A pre-release marker always sorts below a real component.
        (Tok::Pre(_), _) => Ordering::Less,
        (_, Tok::Pre(_)) => Ordering::Greater,
        // Numeric components outrank alphabetic ones (1.2 > 1.b).
        (Tok::Num(_), Tok::Str(_)) => Ordering::Greater,
        (Tok::Str(_), Tok::Num(_)) => Ordering::Less,
    }
}

/// Order two Homebrew version strings.
///
/// Rules that matter: `1.10 > 1.9` (numeric, not lexical), `1.0 > 1.0rc1`
/// (a bare version beats its own pre-releases), `1.0_1 > 1.0` (revision bump).
pub fn version_cmp(a: &str, b: &str) -> Ordering {
    let (ta, ra) = tokenize(a);
    let (tb, rb) = tokenize(b);
    let n = ta.len().max(tb.len());
    for i in 0..n {
        match (ta.get(i), tb.get(i)) {
            (Some(x), Some(y)) => match cmp_tok(x, y) {
                Ordering::Equal => {}
                o => return o,
            },
            // Ran out on one side: trailing pre-release tokens make the *shorter*
            // side greater (1.0 > 1.0rc1); anything else makes it lesser.
            (None, Some(Tok::Pre(_))) => return Ordering::Greater,
            (Some(Tok::Pre(_)), None) => return Ordering::Less,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (None, None) => unreachable!(),
        }
    }
    ra.cmp(&rb)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc(needle: &str, hay: &str) -> Option<i32> {
        score(&fold(needle), &fold(hay), hay.as_bytes())
    }

    #[test]
    fn non_subsequence_does_not_match() {
        assert!(sc("zzz", "ripgrep").is_none());
        assert!(sc("prg", "ripgrep").is_none()); // order matters
        assert!(sc("ripgrepx", "ripgrep").is_none());
    }

    #[test]
    fn exact_prefix_beats_scattered() {
        let exact = sc("rip", "ripgrep").unwrap();
        let scattered = sc("rip", "rust-in-peace").unwrap();
        assert!(exact > scattered, "{exact} !> {scattered}");
    }

    #[test]
    fn boundary_matches_outrank_interior() {
        // "np" hits two word starts here...
        let boundary = sc("np", "nosey-parker").unwrap();
        // ...but is buried mid-token here.
        let interior = sc("np", "unpack").unwrap();
        assert!(boundary > interior, "{boundary} !> {interior}");
    }

    #[test]
    fn shorter_haystack_wins_on_ties() {
        let short = sc("git", "git").unwrap();
        let long = sc("git", "git-delta-extras").unwrap();
        assert!(short > long);
    }

    #[test]
    fn empty_needle_matches_everything() {
        assert_eq!(sc("", "anything"), Some(0));
    }

    #[test]
    fn version_numeric_not_lexical() {
        assert_eq!(version_cmp("1.10", "1.9"), Ordering::Greater);
        assert_eq!(version_cmp("10.0", "9.0"), Ordering::Greater);
        assert_eq!(version_cmp("1.2.3", "1.2.3"), Ordering::Equal);
    }

    #[test]
    fn version_prerelease_below_release() {
        assert_eq!(version_cmp("1.0", "1.0rc1"), Ordering::Greater);
        assert_eq!(version_cmp("1.0beta2", "1.0rc1"), Ordering::Less);
        assert_eq!(version_cmp("1.0alpha1", "1.0beta1"), Ordering::Less);
    }

    #[test]
    fn version_revision_breaks_ties() {
        assert_eq!(version_cmp("1.2.3_1", "1.2.3"), Ordering::Greater);
        assert_eq!(version_cmp("1.2.3_2", "1.2.3_10"), Ordering::Less);
    }

    #[test]
    fn version_real_homebrew_samples() {
        assert_eq!(version_cmp("20260817.0", "20260101.0"), Ordering::Greater);
        assert_eq!(version_cmp("1.3.30-stable", "1.3.9-stable"), Ordering::Greater);
        assert_eq!(version_cmp("2.36.34", "2.36.5"), Ordering::Greater);
    }

    /// Guards the property the UI depends on: one keystroke must rescore the
    /// whole catalog well inside a frame. The bound is deliberately loose (a
    /// loaded machine is still ~10x under it) -- it exists to catch someone
    /// reintroducing per-keystroke allocation or `to_lowercase`, not to measure.
    #[test]
    fn scoring_whole_catalog_is_frame_fast() {
        // Shape matches the real catalog: ~16k entries, name + description.
        let corpus: Vec<(Vec<u8>, String)> = (0..16_291)
            .map(|i| {
                let s = format!("package-name-{i}-tool");
                (fold(&s), s)
            })
            .collect();
        let needle = fold("pnt");

        let t = std::time::Instant::now();
        let hits = corpus
            .iter()
            .filter_map(|(lc, raw)| score(&needle, lc, raw.as_bytes()))
            .count();
        let dt = t.elapsed();

        println!("scored {} entries in {:?} ({} hits)", corpus.len(), dt, hits);
        assert!(hits > 0, "sanity: needle should match this corpus");
        assert!(
            dt < std::time::Duration::from_millis(250),
            "catalog scoring regressed badly: {dt:?}"
        );
    }

    /// Ordering must be a total order, or `sort_by` can panic or corrupt.
    #[test]
    fn version_ordering_is_antisymmetric() {
        let vs = [
            "1.0", "1.0rc1", "1.0_1", "1.10", "1.9", "2", "0.28.0", "20260817.0",
            "1.3.30-stable", "1.2.3_2",
        ];
        for a in vs {
            for b in vs {
                assert_eq!(
                    version_cmp(a, b),
                    version_cmp(b, a).reverse(),
                    "asymmetry between {a} and {b}"
                );
            }
        }
    }
}
