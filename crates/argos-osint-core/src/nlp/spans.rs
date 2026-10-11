//! Explicit coordinate conversion for immutable UTF-8 evidence.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Span {
    pub start_byte: usize,
    pub end_byte: usize,
}
impl Span {
    pub fn slice<'a>(&self, text: &'a str) -> Option<&'a str> {
        if self.start_byte >= self.end_byte {
            return None;
        }
        text.get(self.start_byte..self.end_byte)
    }
}
pub fn utf16_to_byte(text: &str, offset: usize) -> Option<usize> {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units == offset {
            return Some(byte);
        }
        units += ch.len_utf16();
        if units > offset {
            return None;
        }
    }
    (units == offset).then_some(text.len())
}
pub fn scalar_to_byte(text: &str, offset: usize) -> Option<usize> {
    text.char_indices()
        .map(|(b, _)| b)
        .chain(std::iter::once(text.len()))
        .nth(offset)
}
pub fn byte_to_scalar(text: &str, offset: usize) -> Option<usize> {
    text.get(..offset).map(|s| s.chars().count())
}
/// Paragraph ranges include original line endings and whitespace.
pub fn paragraphs(text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut start = 0;
    let mut pos = 0;
    for line in text.split_inclusive('\n') {
        pos += line.len();
        if line.trim().is_empty() && pos > start {
            spans.push(Span {
                start_byte: start,
                end_byte: pos,
            });
            start = pos;
        }
    }
    if start < text.len() {
        spans.push(Span {
            start_byte: start,
            end_byte: text.len(),
        });
    }
    spans
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_coordinates() {
        let s = "😀中e\u{301}\r\n";
        assert_eq!(utf16_to_byte(s, 1), None);
        assert_eq!(utf16_to_byte(s, 2), Some(4));
        for (b, _) in s.char_indices() {
            assert_eq!(scalar_to_byte(s, byte_to_scalar(s, b).unwrap()), Some(b));
        }
        assert_eq!(
            paragraphs(s)
                .iter()
                .map(|r| r.slice(s).unwrap())
                .collect::<String>(),
            s
        );
    }
}
