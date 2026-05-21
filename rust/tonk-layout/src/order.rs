// No wasm-side consumer yet; writer / model wire these in once
// they land.
#![allow(dead_code)]

//! LexoRank-style lexicographic ordering keys.
//!
//! [`between`] returns a string strictly between two others under
//! ASCII byte ordering, with `None` denoting "no bound" on either
//! side. Used as `column.order` and `tile.order` values so a new
//! sibling can be inserted between two neighbours by minting a key
//! that sorts between theirs — repeatedly, indefinitely, with no
//! precision floor.
//!
//! Alphabet: lowercase ASCII `a..=z`. Bytes outside that range are
//! never emitted; callers must pass keys produced by this module (or
//! the empty string, treated identically to `None`).

/// Smallest emittable byte in the alphabet.
const MIN: u8 = b'a';
/// Largest emittable byte in the alphabet.
const MAX: u8 = b'z';

/// Returns a key strictly between `low` and `high` in byte-lex
/// ordering. `None` on either side means "no bound".
///
/// Returns `None` when no key fits — most commonly when the high
/// bound is `Some("a")` (or any leading `'a'` with no tail) since
/// the `[a-z]` alphabet has no char below `'a'`. Also `None` on
/// contract violations (`low >= high`).
pub fn between(low: Option<&str>, high: Option<&str>) -> Option<String> {
    let lo = low.unwrap_or("").as_bytes();
    let bytes = match high {
        Some(h) => {
            if lo >= h.as_bytes() {
                return None;
            }
            bisect(lo, h.as_bytes())?
        }
        None => above(lo),
    };
    Some(String::from_utf8(bytes).expect("alphabet is ASCII"))
}

/// Returns a byte string strictly greater than `low` and strictly
/// less than `high`, or `None` if no such string exists in the
/// finite alphabet.
fn bisect(low: &[u8], high: &[u8]) -> Option<Vec<u8>> {
    let mut result = Vec::new();
    let mut i = 0;
    loop {
        match (low.get(i).copied(), high.get(i).copied()) {
            (Some(lc), Some(hc)) if lc == hc => {
                // Shared char — emit and advance.
                result.push(lc);
                i += 1;
            }
            (Some(lc), Some(hc)) => {
                // lc < hc by the caller's contract.
                if hc - lc > 1 {
                    result.push((lc + hc) / 2);
                    return Some(result);
                }
                // Adjacent. Emit lc; remainder is "strictly above low's
                // tail" (any extension stays below high because we
                // committed to lc < hc at this position).
                result.push(lc);
                result.extend(above(&low[i + 1..]));
                return Some(result);
            }
            (None, Some(hc)) => {
                if hc > MIN {
                    // Room for a char strictly between MIN-1 and hc.
                    // Pick the midpoint of the valid range [MIN, hc-1].
                    result.push((MIN + hc - 1) / 2);
                    return Some(result);
                }
                // hc == MIN. Extending with MIN leaves result < high
                // only when high has further chars; otherwise no key
                // fits (no char below MIN exists in the alphabet).
                if i + 1 < high.len() {
                    result.push(MIN);
                    return Some(result);
                }
                return None;
            }
            (Some(_), None) => {
                // High exhausted while low continues — means high is
                // a prefix of low, so high <= low. Contract violation.
                return None;
            }
            (None, None) => {
                // low == high.
                return None;
            }
        }
    }
}

/// Returns a byte string strictly greater than `low` with no upper
/// bound.
fn above(low: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    let mut i = 0;
    loop {
        let lc = low.get(i).copied().unwrap_or(MIN - 1);
        if lc < MAX {
            // Room above lc within the alphabet. Pick the midpoint of
            // (lc, MAX] — the valid range for a char strictly above lc.
            result.push((lc + 1 + MAX) / 2);
            return result;
        }
        // lc == MAX. Emit MAX and continue — caller's low has a 'z'
        // here, so we must extend further.
        result.push(MAX);
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// Check the result is strictly between the inputs under
    /// byte-lex ordering. `None` on either side means "no bound".
    fn assert_strictly_between(low: Option<&str>, high: Option<&str>, mid: &str) {
        if let Some(l) = low {
            assert!(
                l < mid,
                "result {mid:?} is not strictly greater than low {l:?}",
            );
        }
        if let Some(h) = high {
            assert!(
                mid < h,
                "result {mid:?} is not strictly less than high {h:?}",
            );
        }
    }

    #[dialog_common::test]
    fn it_returns_a_midpoint_between_keys_with_room() {
        // `between` picks a char strictly between `a` and `c`.
        let mid = between(Some("a"), Some("c")).expect("key fits");
        assert_strictly_between(Some("a"), Some("c"), &mid);
        assert_eq!(mid, "b");
    }

    #[dialog_common::test]
    fn it_extends_the_key_when_neighbours_are_adjacent() {
        // No char fits strictly between `a` and `b` at one position;
        // the key must lengthen.
        let mid = between(Some("a"), Some("b")).expect("key fits");
        assert_strictly_between(Some("a"), Some("b"), &mid);
        assert!(mid.len() > 1, "expected extension, got {mid:?}");
        assert!(mid.starts_with('a'));
    }

    #[dialog_common::test]
    fn it_returns_a_first_key_when_both_bounds_are_open() {
        let mid = between(None, None).expect("key fits");
        assert!(!mid.is_empty());
        assert!(mid.chars().all(|c: char| c.is_ascii_lowercase()));
    }

    #[dialog_common::test]
    fn it_returns_a_key_below_high_when_low_is_open() {
        let mid = between(None, Some("m")).expect("key fits");
        assert_strictly_between(None, Some("m"), &mid);
    }

    #[dialog_common::test]
    fn it_returns_a_key_above_low_when_high_is_open() {
        let mid = between(Some("m"), None).expect("key fits");
        assert_strictly_between(Some("m"), None, &mid);
    }

    #[dialog_common::test]
    fn it_bisects_into_a_long_shared_prefix() {
        // The keys share a 10-char prefix and diverge after; `between`
        // must walk past the prefix and place a char in the
        // diverging position.
        let lo = "aaaaaaaaaa";
        let hi = "aaaaaaaaab";
        let mid = between(Some(lo), Some(hi)).expect("key fits");
        assert_strictly_between(Some(lo), Some(hi), &mid);
        assert!(mid.starts_with("aaaaaaaaaa"));
    }

    #[dialog_common::test]
    fn it_extends_indefinitely_above_a_low_anchor() {
        // Repeated `between(prev, None)` should keep producing larger
        // keys with bounded length growth — a numerically-indexed
        // scheme would run out of precision long before this.
        let mut lo = String::from("a");
        for _ in 0..200 {
            let next = between(Some(&lo), None).expect("key fits");
            assert!(next > lo, "key {next:?} not above {lo:?}");
            lo = next;
        }
        // Each step advances by roughly log2(alphabet) bits; with a
        // 26-char alphabet, 200 steps stay well under 100 chars.
        assert!(lo.len() < 100, "subdivision blew up: final key {lo:?}");
    }

    #[dialog_common::test]
    fn it_subdivides_repeatedly_between_two_neighbours() {
        // Repeatedly bisecting between a low and high anchor (each
        // step replacing the low with the result) should also stay
        // bounded.
        let mut lo = String::from("a");
        let hi = String::from("b");
        for _ in 0..200 {
            let next = between(Some(&lo), Some(&hi)).expect("key fits");
            assert!(lo < next && next < hi, "key {next:?} out of bounds");
            lo = next;
        }
        assert!(lo.len() < 100, "subdivision blew up: {lo:?}");
    }

    #[dialog_common::test]
    fn it_returns_none_below_a_single_min_char() {
        // No string fits strictly between "" (or any lower bound)
        // and "a" — the alphabet has no char below MIN.
        assert!(between(None, Some("a")).is_none());
    }

    #[dialog_common::test]
    fn it_returns_none_when_low_equals_high() {
        assert!(between(Some("m"), Some("m")).is_none());
    }

    #[dialog_common::test]
    fn it_returns_none_when_low_is_greater_than_high() {
        assert!(between(Some("z"), Some("a")).is_none());
    }
}
