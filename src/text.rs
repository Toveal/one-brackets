//! Text of strings and atoms: Unicode that may hold unpaired surrogates.

use std::borrow::Cow;
use std::fmt;

/// Text of a string or an atom.
///
/// The platform keeps text as UTF-16 code units and writes each surrogate of a quoted string
/// as an escape `"\xxxx`, so a string may hold an unpaired surrogate. `Text` keeps such
/// strings without losses; [`as_str`](Text::as_str) gives `None` for them and
/// [`to_string_lossy`](Text::to_string_lossy) replaces unpaired surrogates with U+FFFD.
///
/// Stored as [WTF-8](https://simonsapin.github.io/wtf-8/): UTF-8 that also encodes unpaired
/// surrogates. Text without them is plain UTF-8.
#[derive(Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Text {
    bytes: Vec<u8>,
}

impl Text {
    pub fn new() -> Self {
        Self::default()
    }

    /// Text from UTF-16 code units; unpaired surrogates are kept.
    pub fn from_utf16(units: &[u16]) -> Self {
        let mut bytes = Vec::with_capacity(units.len());
        for &unit in units {
            push_code_point(&mut bytes, u32::from(unit));
        }
        Self { bytes }
    }

    /// WTF-8 bytes produced by this module or validated as UTF-8.
    pub(crate) fn from_wtf8(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// The text, unless it holds an unpaired surrogate.
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.bytes).ok()
    }

    /// The text with unpaired surrogates replaced by U+FFFD.
    pub fn to_string_lossy(&self) -> Cow<'_, str> {
        match self.as_str() {
            Some(s) => Cow::Borrowed(s),
            None => Cow::Owned(
                self.code_points()
                    .map(|c| char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER))
                    .collect(),
            ),
        }
    }

    /// The text as a `String`; `Err` gives it back if it holds an unpaired surrogate.
    pub fn into_string(self) -> Result<String, Text> {
        String::from_utf8(self.bytes).map_err(|e| Text {
            bytes: e.into_bytes(),
        })
    }

    /// UTF-16 code units, as the platform keeps the text.
    pub fn encode_utf16(&self) -> impl Iterator<Item = u16> + '_ {
        self.code_points().flat_map(|c| {
            let (first, second) = match c {
                0x10000.. => {
                    let c = c - 0x10000;
                    (0xD800 | (c >> 10) as u16, Some(0xDC00 | (c & 0x3FF) as u16))
                }
                _ => (c as u16, None),
            };
            std::iter::once(first).chain(second)
        })
    }

    /// Unicode code points; unpaired surrogates come as themselves (`0xD800..=0xDFFF`).
    pub fn code_points(&self) -> impl Iterator<Item = u32> + '_ {
        let bytes = &self.bytes;
        let mut i = 0;
        std::iter::from_fn(move || {
            let first = *bytes.get(i)?;
            let (len, mut c) = match first {
                0..=0x7F => (1, u32::from(first)),
                0xC0..=0xDF => (2, u32::from(first & 0x1F)),
                0xE0..=0xEF => (3, u32::from(first & 0x0F)),
                _ => (4, u32::from(first & 0x07)),
            };
            for &b in &bytes[i + 1..i + len] {
                c = c << 6 | u32::from(b & 0x3F);
            }
            i += len;
            Some(c)
        })
    }

    /// WTF-8 bytes: UTF-8 if the text has no unpaired surrogates.
    pub fn as_wtf8(&self) -> &[u8] {
        &self.bytes
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Length in WTF-8 bytes.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }
}

/// Appends a code point, joining a low surrogate with a high one before it.
pub(crate) fn push_code_point(bytes: &mut Vec<u8>, c: u32) {
    let mut c = c;
    if let 0xDC00..=0xDFFF = c
        && let Some(high) = trailing_high_surrogate(bytes)
    {
        bytes.truncate(bytes.len() - 3);
        c = 0x10000 + ((high - 0xD800) << 10) + (c - 0xDC00);
    }
    match char::from_u32(c) {
        Some(ch) => bytes.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes()),
        // A surrogate: the 3-byte form UTF-8 would have for it.
        None => bytes.extend_from_slice(&[
            0xE0 | (c >> 12) as u8,
            0x80 | (c >> 6 & 0x3F) as u8,
            0x80 | (c & 0x3F) as u8,
        ]),
    }
}

fn trailing_high_surrogate(bytes: &[u8]) -> Option<u32> {
    match bytes {
        [.., 0xED, b1 @ 0xA0..=0xAF, b2] => Some(surrogate(*b1, *b2)),
        _ => None,
    }
}

/// The surrogate encoded as `ED b1 b2`.
fn surrogate(b1: u8, b2: u8) -> u32 {
    0xD000 | u32::from(b1 & 0x3F) << 6 | u32::from(b2 & 0x3F)
}

impl From<String> for Text {
    fn from(s: String) -> Self {
        Self {
            bytes: s.into_bytes(),
        }
    }
}

impl From<&str> for Text {
    fn from(s: &str) -> Self {
        Self::from(s.to_owned())
    }
}

impl From<char> for Text {
    fn from(c: char) -> Self {
        Self::from(c.to_string())
    }
}

impl From<&String> for Text {
    fn from(s: &String) -> Self {
        Self::from(s.as_str())
    }
}

impl From<Cow<'_, str>> for Text {
    fn from(s: Cow<'_, str>) -> Self {
        Self::from(s.into_owned())
    }
}

impl From<&Text> for Text {
    fn from(text: &Text) -> Self {
        text.clone()
    }
}

impl PartialEq<str> for Text {
    fn eq(&self, other: &str) -> bool {
        self.bytes == other.as_bytes()
    }
}

impl PartialEq<&str> for Text {
    fn eq(&self, other: &&str) -> bool {
        self.bytes == other.as_bytes()
    }
}

impl PartialEq<String> for Text {
    fn eq(&self, other: &String) -> bool {
        self.bytes == other.as_bytes()
    }
}

impl fmt::Debug for Text {
    /// Like `str`; unpaired surrogates are shown as `\u{d800}`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(s) = self.as_str() {
            return fmt::Debug::fmt(s, f);
        }
        f.write_str("\"")?;
        for c in self.code_points() {
            match char::from_u32(c) {
                Some('\'') => f.write_str("'")?,
                Some(ch) => write!(f, "{}", ch.escape_debug())?,
                None => write!(f, "\\u{{{c:x}}}")?,
            }
        }
        f.write_str("\"")
    }
}

impl fmt::Display for Text {
    /// Unpaired surrogates are shown as U+FFFD.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_lossy())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_unpaired_surrogates() {
        let text = Text::from_utf16(&[0x61, 0xD83D, 0xDE00, 0xD800, 0x62, 0xDC00]);
        assert_eq!(text.as_str(), None);
        assert_eq!(text.to_string_lossy(), "a😀\u{fffd}b\u{fffd}");
        assert_eq!(
            text.encode_utf16().collect::<Vec<_>>(),
            [0x61, 0xD83D, 0xDE00, 0xD800, 0x62, 0xDC00]
        );
        assert_eq!(format!("{text:?}"), r#""a😀\u{d800}b\u{dc00}""#);
        assert_eq!(text.to_string(), "a😀\u{fffd}b\u{fffd}");
        assert!(text.clone().into_string().is_err());
    }

    #[test]
    fn pairs_become_characters() {
        let text = Text::from_utf16(&[0xD83D, 0xDE00]);
        assert_eq!(text.as_str(), Some("😀"));
        assert_eq!(text, "😀");

        let mut bytes = Vec::new();
        push_code_point(&mut bytes, 0xD83D);
        push_code_point(&mut bytes, 0x78);
        assert_eq!(Text::from_wtf8(bytes).to_string_lossy(), "\u{fffd}x");
    }

    #[test]
    fn debug_matches_str() {
        assert_eq!(
            format!("{:?}", Text::from("a \"b\" 'c'\n")),
            r#""a \"b\" 'c'\n""#
        );
    }
}
