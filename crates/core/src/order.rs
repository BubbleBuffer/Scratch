//! Sibling order keys for pages: strings over base-62 digits (`0-9A-Za-z`,
//! which sort in ASCII order) compared lexicographically. A key never ends in
//! `0`, so there is always room between two different keys.
//!
//! Appending increments a 4-digit prefix (so 10,000 appends stay 4 to 5
//! characters long); inserting between two keys takes a midpoint.

const DIGITS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const BASE: u8 = 62;
const APPEND_WIDTH: usize = 4;

fn digit(c: u8) -> u8 {
    DIGITS.iter().position(|&d| d == c).unwrap_or(0) as u8
}

fn to_digits(s: &str) -> Vec<u8> {
    s.bytes().map(digit).collect()
}

fn to_key(d: &[u8]) -> String {
    d.iter().map(|&x| DIGITS[x as usize] as char).collect()
}

/// A key strictly between `a` and `b` (`None` = open end). Keys that are
/// equal or out of order (from concurrent edits) yield a key after `a`.
pub(crate) fn between(a: Option<&str>, b: Option<&str>) -> String {
    match (a, b) {
        (None, None) => "V".to_string(),
        (Some(a), None) => after(a),
        (a, Some(b)) => {
            let a = a.unwrap_or("");
            if !b.is_empty() && a < b {
                to_key(&midpoint(&to_digits(a), Some(&to_digits(b))))
            } else {
                after(a)
            }
        }
    }
}

/// A short key after `a`: its first 4 digits (padded with zeros) plus one.
fn after(a: &str) -> String {
    let mut d = to_digits(a);
    d.truncate(APPEND_WIDTH);
    d.resize(APPEND_WIDTH, 0);
    loop {
        // Increment as a base-62 number; on overflow extend `a` instead.
        let mut i = APPEND_WIDTH;
        loop {
            if i == 0 {
                return to_key(&midpoint(&to_digits(a), None));
            }
            i -= 1;
            if d[i] + 1 < BASE {
                d[i] += 1;
                break;
            }
            d[i] = 0;
        }
        if *d.last().unwrap() != 0 {
            return to_key(&d);
        }
    }
}

/// Midpoint of digit strings `a < b` (`None` = past the end), no trailing zero.
fn midpoint(a: &[u8], b: Option<&[u8]>) -> Vec<u8> {
    if let Some(b) = b {
        let mut n = 0;
        while n < b.len() && a.get(n).copied().unwrap_or(0) == b[n] {
            n += 1;
        }
        if n > 0 {
            let mut out = b[..n].to_vec();
            out.extend(midpoint(a.get(n..).unwrap_or(&[]), Some(&b[n..])));
            return out;
        }
    }
    let da = a.first().copied().unwrap_or(0);
    let db = b
        .map(|b| b.first().copied().unwrap_or(BASE))
        .unwrap_or(BASE);
    if db - da > 1 {
        vec![(da + db) / 2]
    } else if let Some(b) = b.filter(|b| b.len() > 1) {
        vec![b[0]]
    } else {
        let mut out = vec![da];
        out.extend(midpoint(a.get(1..).unwrap_or(&[]), None));
        out
    }
}

/// `n` evenly spread fresh keys, for re-keying siblings whose keys collide.
pub(crate) fn spread(n: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(n);
    let mut last: Option<String> = None;
    for _ in 0..n {
        let k = between(last.as_deref(), None);
        out.push(k.clone());
        last = Some(k);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_stay_ordered_and_short() {
        let mut keys = vec![between(None, None)];
        for _ in 0..10_000 {
            let k = between(keys.last().map(String::as_str), None);
            assert!(k > *keys.last().unwrap());
            keys.push(k);
        }
        assert!(keys.iter().all(|k| k.len() <= 5), "{:?}", keys.last());
        // Repeatedly inserting at the front and in the middle.
        let mut first = keys[0].clone();
        for _ in 0..200 {
            let k = between(None, Some(&first));
            assert!(k < first && !k.is_empty() && !k.ends_with('0'), "{k}");
            first = k;
        }
        let (mut lo, hi) = (keys[5].clone(), keys[6].clone());
        for _ in 0..200 {
            let k = between(Some(&lo), Some(&hi));
            assert!(lo < k && k < hi, "{lo} {k} {hi}");
            lo = k;
        }
        assert!(between(Some("V"), Some("V")).as_str() > "V");
        assert!(between(Some("W"), Some("V")).as_str() > "W");
        assert!(between(Some(""), Some("")).as_str() > "");
    }
}
