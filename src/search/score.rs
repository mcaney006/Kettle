const SCORE_MATCH: i32 = 16;
const BONUS_BOUNDARY: i32 = 8;
const BONUS_CAMEL: i32 = 7;
const BONUS_CONSECUTIVE: i32 = 8;
const BONUS_FIRST_CHAR: i32 = 12;
const PENALTY_GAP_START: i32 = -3;
const PENALTY_GAP_EXTEND: i32 = -1;

#[inline]
fn is_separator(byte: u8) -> bool {
    matches!(byte, b'-' | b'_' | b'.' | b'/' | b'@' | b' ' | b'+')
}

#[inline]
fn boundary_bonus(haystack: &[u8], index: usize) -> i32 {
    if index == 0 {
        return BONUS_BOUNDARY + BONUS_FIRST_CHAR;
    }
    let previous = haystack[index - 1];
    if is_separator(previous) {
        BONUS_BOUNDARY
    } else if previous.is_ascii_lowercase() && haystack[index].is_ascii_uppercase() {
        BONUS_CAMEL
    } else {
        0
    }
}

pub fn score(needle: &[u8], haystack: &[u8], raw: &[u8]) -> Option<i32> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > haystack.len() {
        return None;
    }
    let needle_len = needle.len().min(64);

    let mut matched = 0;
    let mut end = None;
    for (index, &byte) in haystack.iter().enumerate() {
        if byte == needle[matched] {
            matched += 1;
            if matched == needle_len {
                end = Some(index);
                break;
            }
        }
    }
    let end = end?;

    matched = needle_len;
    let mut start = 0;
    for index in (0..=end).rev() {
        if haystack[index] == needle[matched - 1] {
            matched -= 1;
            if matched == 0 {
                start = index;
                break;
            }
        }
    }

    let mut positions = [0; 64];
    matched = 0;
    for (index, &byte) in haystack.iter().enumerate().take(end + 1).skip(start) {
        if matched < needle_len && byte == needle[matched] {
            positions[matched] = index;
            matched += 1;
        }
    }

    let mut total = 0;
    let mut previous = None;
    let mut in_gap = false;
    for &index in &positions[..needle_len] {
        total += SCORE_MATCH;
        match previous {
            Some(previous_index) if index == previous_index + 1 => {
                total += BONUS_CONSECUTIVE;
                in_gap = false;
            }
            Some(_) => {
                total += if in_gap {
                    PENALTY_GAP_EXTEND
                } else {
                    PENALTY_GAP_START
                };
                in_gap = true;
                total += boundary_bonus(raw, index);
            }
            None => total += boundary_bonus(raw, index),
        }
        previous = Some(index);
    }
    total -= haystack.len() as i32 / 8;
    Some(total)
}

pub fn fold(value: &str) -> Vec<u8> {
    value
        .bytes()
        .map(|byte| byte.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scored(needle: &str, haystack: &str) -> Option<i32> {
        score(&fold(needle), &fold(haystack), haystack.as_bytes())
    }

    #[test]
    fn search_is_deterministic_and_boundary_aware() {
        assert!(scored("zzz", "ripgrep").is_none());
        assert!(scored("rip", "ripgrep").unwrap() > scored("rip", "rust-in-peace").unwrap());
        assert!(scored("np", "nosey-parker").unwrap() > scored("np", "unpack").unwrap());
        assert_eq!(scored("", "anything"), Some(0));
    }
}
