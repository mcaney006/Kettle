use std::cmp::Ordering;

#[derive(Debug, Eq, PartialEq)]
enum Token {
    Pre(u8),
    Num(String),
    Text(String),
}

fn pre_rank(value: &str) -> Option<u8> {
    match value {
        "alpha" | "a" => Some(0),
        "beta" | "b" => Some(1),
        "pre" => Some(2),
        "rc" => Some(3),
        _ => None,
    }
}

fn numeric(value: &str) -> String {
    let trimmed = value.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn tokenize(value: &str) -> (Vec<Token>, String) {
    let (base, revision) = match value.rsplit_once('_') {
        Some((base, revision))
            if !revision.is_empty() && revision.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            (base, numeric(revision))
        }
        _ => (value, "0".to_owned()),
    };

    let mut tokens = Vec::new();
    let bytes = base.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_alphanumeric() {
            index += 1;
            continue;
        }
        let start = index;
        let is_numeric = bytes[index].is_ascii_digit();
        while index < bytes.len()
            && bytes[index].is_ascii_alphanumeric()
            && bytes[index].is_ascii_digit() == is_numeric
        {
            index += 1;
        }
        let part = &base[start..index];
        tokens.push(if is_numeric {
            Token::Num(numeric(part))
        } else {
            let folded = part.to_ascii_lowercase();
            pre_rank(&folded).map_or(Token::Text(folded), Token::Pre)
        });
    }
    (tokens, revision)
}

fn cmp_numeric(left: &str, right: &str) -> Ordering {
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

fn cmp_token(left: &Token, right: &Token) -> Ordering {
    match (left, right) {
        (Token::Num(left), Token::Num(right)) => cmp_numeric(left, right),
        (Token::Text(left), Token::Text(right)) => left.cmp(right),
        (Token::Pre(left), Token::Pre(right)) => left.cmp(right),
        (Token::Pre(_), _) => Ordering::Less,
        (_, Token::Pre(_)) => Ordering::Greater,
        (Token::Num(_), Token::Text(_)) => Ordering::Greater,
        (Token::Text(_), Token::Num(_)) => Ordering::Less,
    }
}

/// Kettle's local ordering for choosing the newest installed directory.
/// Homebrew remains authoritative for whether a package is actually outdated.
pub fn version_cmp(left: &str, right: &str) -> Ordering {
    let (left_tokens, left_revision) = tokenize(left);
    let (right_tokens, right_revision) = tokenize(right);
    for index in 0..left_tokens.len().max(right_tokens.len()) {
        match (left_tokens.get(index), right_tokens.get(index)) {
            (Some(left), Some(right)) => {
                let ordering = cmp_token(left, right);
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            (None, Some(Token::Pre(_))) => return Ordering::Greater,
            (Some(Token::Pre(_)), None) => return Ordering::Less,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (None, None) => break,
        }
    }
    cmp_numeric(&left_revision, &right_revision)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const REAL: &[&str] = &[
        "0.28.0",
        "1.0",
        "1.0rc1",
        "1.0_1",
        "1.2.3_10",
        "1.3.30-stable",
        "2.36.34",
        "26.825.41651",
        "20260817.0",
        "999999999999999999999999999999999999.2",
        "1..2---rc01",
    ];

    #[test]
    fn representative_ordering() {
        assert_eq!(version_cmp("1.10", "1.9"), Ordering::Greater);
        assert_eq!(version_cmp("1.0", "1.0rc1"), Ordering::Greater);
        assert_eq!(version_cmp("1.0_2", "1.0_10"), Ordering::Less);
        assert_eq!(
            version_cmp(
                "999999999999999999999999999999999999.2",
                "888888888888888888888888888888888888.9",
            ),
            Ordering::Greater
        );
        assert_eq!(
            version_cmp("1.3.30-stable", "1.3.9-stable"),
            Ordering::Greater
        );
    }

    #[test]
    fn real_corpus_is_total_and_transitive() {
        for &a in REAL {
            for &b in REAL {
                assert_eq!(version_cmp(a, b), version_cmp(b, a).reverse());
                for &c in REAL {
                    if version_cmp(a, b) != Ordering::Greater
                        && version_cmp(b, c) != Ordering::Greater
                    {
                        assert_ne!(version_cmp(a, c), Ordering::Greater, "{a} <= {b} <= {c}");
                    }
                }
            }
        }
    }

    proptest! {
        #[test]
        fn antisymmetric_for_generated_versions(
            a in "[0-9A-Za-z._+\\-]{0,48}",
            b in "[0-9A-Za-z._+\\-]{0,48}",
        ) {
            prop_assert_eq!(version_cmp(&a, &b), version_cmp(&b, &a).reverse());
        }

        #[test]
        fn transitive_for_generated_versions(
            mut versions in proptest::collection::vec("[0-9A-Za-z._+\\-]{0,32}", 3),
        ) {
            versions.sort_by(|a, b| version_cmp(a, b));
            prop_assert_ne!(version_cmp(&versions[0], &versions[2]), Ordering::Greater);
        }
    }
}
