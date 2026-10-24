//! Typed key codec for compound record keys.
//!
//! Every component is kept and length-prefixed (`<len>:<bytes>` joined by
//! `|`), so no separator inside a component (colons, slashes, pipes) can make
//! two different keys collide, and [`decode`] recovers the exact parts.

/// Encode `parts` into one stable record key.
pub fn encode(parts: &[&str]) -> String {
    let mut out = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            out.push('|');
        }
        out.push_str(&part.len().to_string());
        out.push(':');
        out.push_str(part);
    }
    out
}

/// Decode a key made by [`encode`]; `None` for anything else.
pub fn decode(key: &str) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut rest = key;
    loop {
        let colon = rest.find(':')?;
        let len: usize = rest[..colon].parse().ok()?;
        let body = rest.get(colon + 1..colon + 1 + len)?;
        parts.push(body.to_string());
        rest = &rest[colon + 1 + len..];
        if rest.is_empty() {
            return Some(parts);
        }
        rest = rest.strip_prefix('|')?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_awkward_components() {
        let cases: &[&[&str]] = &[
            &["a"],
            &["", ""],
            &["job:1", "stage/2", "x|y"],
            &["3:abc", "|", "::"],
            &["naïve", "日本"],
        ];
        for parts in cases {
            let key = encode(parts);
            assert_eq!(decode(&key).unwrap(), *parts, "{key}");
        }
    }

    #[test]
    fn separators_inside_components_do_not_collide() {
        assert_ne!(encode(&["a:b", "c"]), encode(&["a", "b:c"]));
        assert_ne!(encode(&["a|b"]), encode(&["a", "b"]));
        assert_ne!(encode(&["1:a"]), encode(&["1", "a"]));
    }

    #[test]
    fn rejects_malformed_keys() {
        for bad in ["", "x", "3:ab", "1:a|", "1:ab", "a:b"] {
            assert!(decode(bad).is_none(), "{bad}");
        }
    }
}
