//! INFO section codec (SPEC.md §2.1): deterministic JSON metadata.
//!
//! The JSON dialect is a strict subset: a single object, keys sorted
//! lexicographically, no whitespace, integer numbers only, minimal string escaping.
//! The decoder is a hand-rolled strict parser (no dependency) that rejects any
//! deviation, including unknown keys and trailing data.

use std::collections::BTreeSet;

use crate::error::{Error, Result};
use crate::util::{Cursor, Writer};

/// The format version the INFO section must declare (SPEC.md §2.1).
pub const INFO_FORMAT_VERSION: u16 = 1;

/// Length of the hex `document_id` (16 bytes = 32 hex chars).
pub const DOCUMENT_ID_HEX_LEN: usize = 32;

/// Length of the hex `source_digest` (SHA-256 = 32 bytes = 64 hex chars).
pub const SOURCE_DIGEST_HEX_LEN: usize = 64;

/// Limit on the INFO JSON size (defensive; metadata is small).
const MAX_INFO_LEN: usize = 1 << 20;

/// Package metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    /// Must equal [`INFO_FORMAT_VERSION`].
    pub format_version: u16,
    /// Compiler name.
    pub generator: String,
    /// Compiler semantic version.
    pub generator_version: String,
    /// Hex SHA-256 of the normalized source input (64 hex chars).
    pub source_digest: String,
    /// Content-addressed document id (32 hex chars).
    pub document_id: String,
    /// Optional document title.
    pub title: Option<String>,
    /// Optional BCP 47 language tag.
    pub lang: Option<String>,
    /// CHNK record count.
    pub chunk_count: u32,
    /// STYL record count.
    pub style_count: u32,
    /// CONT payload count.
    pub content_count: u32,
    /// GLYF atlas count.
    pub atlas_count: u32,
    /// IMGS image count.
    pub image_count: u32,
}

/// A JSON value in the INFO subset.
enum JsonValue {
    Num(u64),
    Str(String),
}

impl Info {
    /// Encode to the deterministic JSON byte form.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(b'{');
        let mut first = true;
        for (key, value) in self.ordered_entries() {
            if !first {
                w.u8(b',');
            }
            first = false;
            write_json_string(&mut w, key);
            w.u8(b':');
            match value {
                JsonValue::Num(n) => w.bytes(n.to_string().as_bytes()),
                JsonValue::Str(s) => write_json_string(&mut w, &s),
            }
        }
        w.u8(b'}');
        w.into_bytes()
    }

    /// Decode and strictly validate the INFO payload.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_INFO_LEN {
            return Err(Error::LimitExceeded {
                what: "INFO payload",
                value: bytes.len() as u64,
                limit: MAX_INFO_LEN as u64,
            });
        }
        let mut cursor = Cursor::new(bytes);
        let object = parse_object(&mut cursor)?;
        cursor.finish("INFO payload")?;

        let mut seen = BTreeSet::new();
        let mut numbers = BTreeSet::new();
        let mut strings = BTreeSet::new();
        for (key, value) in &object {
            if !seen.insert(key.as_str()) {
                return Err(Error::UnknownValue {
                    what: "duplicate INFO key",
                    value: 0,
                });
            }
            match value {
                JsonValue::Num(_) => {
                    numbers.insert(key.as_str());
                }
                JsonValue::Str(_) => {
                    strings.insert(key.as_str());
                }
            }
        }

        let get_num = |key: &str| -> Result<u64> {
            object
                .iter()
                .find(|(k, _)| *k == key)
                .and_then(|(_, v)| match v {
                    JsonValue::Num(n) => Some(*n),
                    _ => None,
                })
                .ok_or(Error::UnknownValue {
                    what: "INFO key",
                    value: 0,
                })
        };
        let get_str = |key: &str| -> Result<String> {
            object
                .iter()
                .find(|(k, _)| *k == key)
                .and_then(|(_, v)| match v {
                    JsonValue::Str(s) => Some(s.clone()),
                    _ => None,
                })
                .ok_or(Error::UnknownValue {
                    what: "INFO key",
                    value: 0,
                })
        };
        let optional_str = |key: &str| -> Result<Option<String>> {
            if object.iter().any(|(k, _)| *k == key) {
                Ok(Some(get_str(key)?))
            } else {
                Ok(None)
            }
        };

        let version_value = get_num("format_version")?;
        let format_version = u16::try_from(version_value).map_err(|_| Error::LimitExceeded {
            what: "format_version",
            value: version_value,
            limit: u64::from(u16::MAX),
        })?;
        if format_version != INFO_FORMAT_VERSION {
            return Err(Error::UnsupportedVersion(format_version));
        }
        let generator = get_str("generator")?;
        let generator_version = get_str("generator_version")?;
        let source_digest = get_str("source_digest")?;
        let document_id = get_str("document_id")?;
        let title = optional_str("title")?;
        let lang = optional_str("lang")?;

        if source_digest.len() != SOURCE_DIGEST_HEX_LEN || !is_lower_hex(&source_digest) {
            return Err(Error::UnknownValue {
                what: "INFO source_digest",
                value: 0,
            });
        }
        if document_id.len() != DOCUMENT_ID_HEX_LEN || !is_lower_hex(&document_id) {
            return Err(Error::UnknownValue {
                what: "INFO document_id",
                value: 0,
            });
        }

        let count = |key: &'static str| -> Result<u32> {
            let value = get_num(key)?;
            u32::try_from(value).map_err(|_| Error::LimitExceeded {
                what: key,
                value,
                limit: u64::from(u32::MAX),
            })
        };

        Ok(Self {
            format_version,
            generator,
            generator_version,
            source_digest,
            document_id,
            title,
            lang,
            chunk_count: count("chunk_count")?,
            style_count: count("style_count")?,
            content_count: count("content_count")?,
            atlas_count: count("atlas_count")?,
            image_count: count("image_count")?,
        })
    }

    /// The fields in lexicographically sorted key order (writer discipline).
    fn ordered_entries(&self) -> Vec<(&'static str, JsonValue)> {
        let mut entries = vec![("atlas_count", JsonValue::Num(u64::from(self.atlas_count)))];
        entries.push(("chunk_count", JsonValue::Num(u64::from(self.chunk_count))));
        entries.push((
            "content_count",
            JsonValue::Num(u64::from(self.content_count)),
        ));
        entries.push(("document_id", JsonValue::Str(self.document_id.clone())));
        entries.push((
            "format_version",
            JsonValue::Num(u64::from(self.format_version)),
        ));
        entries.push(("generator", JsonValue::Str(self.generator.clone())));
        entries.push((
            "generator_version",
            JsonValue::Str(self.generator_version.clone()),
        ));
        entries.push(("image_count", JsonValue::Num(u64::from(self.image_count))));
        if let Some(lang) = &self.lang {
            entries.push(("lang", JsonValue::Str(lang.clone())));
        }
        entries.push(("source_digest", JsonValue::Str(self.source_digest.clone())));
        entries.push(("style_count", JsonValue::Num(u64::from(self.style_count))));
        if let Some(title) = &self.title {
            entries.push(("title", JsonValue::Str(title.clone())));
        }
        entries
    }
}

/// Encode a JSON string with minimal escaping.
fn write_json_string(w: &mut Writer, value: &str) {
    w.u8(b'"');
    for c in value.chars() {
        match c {
            '"' => w.bytes(b"\\\""),
            '\\' => w.bytes(b"\\\\"),
            '\n' => w.bytes(b"\\n"),
            '\r' => w.bytes(b"\\r"),
            '\t' => w.bytes(b"\\t"),
            '\u{08}' => w.bytes(b"\\b"),
            '\u{0C}' => w.bytes(b"\\f"),
            c if (c as u32) < 0x20 => {
                w.bytes(format!("\\u{:04x}", c as u32).as_bytes());
            }
            c => {
                let mut buf = [0_u8; 4];
                w.bytes(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    w.u8(b'"');
}

/// Parse a JSON object (no whitespace allowed).
fn parse_object(c: &mut Cursor<'_>) -> Result<Vec<(String, JsonValue)>> {
    if c.u8("INFO object start")? != b'{' {
        return Err(Error::UnknownValue {
            what: "INFO object start",
            value: 0,
        });
    }
    let mut out: Vec<(String, JsonValue)> = Vec::new();
    loop {
        match c.u8("INFO object")? {
            b'}' => break,
            b',' => continue,
            b'"' => {
                let key = parse_string(c)?;
                if c.u8("INFO key separator")? != b':' {
                    return Err(Error::UnknownValue {
                        what: "INFO key separator",
                        value: 0,
                    });
                }
                let value = parse_value(c)?;
                out.push((key, value));
            }
            other => {
                return Err(Error::UnknownValue {
                    what: "INFO object element",
                    value: u64::from(other),
                })
            }
        }
    }
    Ok(out)
}

/// Parse a JSON value: string or non-negative integer.
fn parse_value(c: &mut Cursor<'_>) -> Result<JsonValue> {
    match c.u8("INFO value start")? {
        b'"' => Ok(JsonValue::Str(parse_string(c)?)),
        first @ b'0'..=b'9' => {
            // `first` is the consumed lead digit, bound directly by the pattern.
            let mut n = u64::from(first - b'0');
            loop {
                match c.peek() {
                    Some(b'0'..=b'9') => {
                        let digit = u64::from(c.u8("INFO number")? - b'0');
                        n = n.checked_mul(10).and_then(|v| v.checked_add(digit)).ok_or(
                            Error::LimitExceeded {
                                what: "INFO number",
                                value: u64::MAX,
                                limit: u64::from(u32::MAX),
                            },
                        )?;
                    }
                    Some(b',') | Some(b'}') => break,
                    Some(_) => {
                        return Err(Error::UnknownValue {
                            what: "INFO number terminator",
                            value: 0,
                        })
                    }
                    None => {
                        return Err(Error::UnexpectedEof {
                            what: "INFO number",
                        })
                    }
                }
            }
            Ok(JsonValue::Num(n))
        }
        other => Err(Error::UnknownValue {
            what: "INFO value start",
            value: u64::from(other),
        }),
    }
}

/// Parse a JSON string (opening quote already consumed), decoding escapes.
fn parse_string(c: &mut Cursor<'_>) -> Result<String> {
    let mut out = String::new();
    loop {
        let byte = c.u8("INFO string")?;
        match byte {
            b'"' => break,
            b'\\' => {
                let esc = c.u8("INFO string escape")?;
                match esc {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{08}'),
                    b'f' => out.push('\u{0C}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let hex = c.take(4, "INFO unicode escape")?;
                        let digits = core::str::from_utf8(hex).map_err(|_| Error::InvalidUtf8)?;
                        let value =
                            u32::from_str_radix(digits, 16).map_err(|_| Error::UnknownValue {
                                what: "INFO unicode escape",
                                value: 0,
                            })?;
                        let ch = char::from_u32(value).ok_or(Error::UnknownValue {
                            what: "INFO unicode escape",
                            value: u64::from(value),
                        })?;
                        out.push(ch);
                    }
                    _ => {
                        return Err(Error::UnknownValue {
                            what: "INFO escape",
                            value: u64::from(esc),
                        })
                    }
                }
            }
            b'\x00'..=b'\x1F' => {
                return Err(Error::UnknownValue {
                    what: "INFO raw control char",
                    value: u64::from(byte),
                });
            }
            _ => {
                // The lead byte was already consumed; read only the continuation
                // bytes and reassemble the full UTF-8 sequence.
                let len = utf8_len(byte).ok_or(Error::InvalidUtf8)?;
                if len == 1 {
                    out.push(char::from(byte));
                } else {
                    let cont = c.take(len - 1, "INFO string char")?;
                    // Local fixed-size buffer: `len` is 2..=4, so these accesses are
                    // provably in bounds.
                    #[allow(clippy::indexing_slicing)]
                    {
                        let mut seq = [0_u8; 4];
                        seq[0] = byte;
                        seq[1..len].copy_from_slice(cont);
                        let s =
                            core::str::from_utf8(&seq[..len]).map_err(|_| Error::InvalidUtf8)?;
                        out.push_str(s);
                    }
                }
            }
        }
    }
    Ok(out)
}

/// The byte length of a UTF-8 sequence given its lead byte.
fn utf8_len(lead: u8) -> Option<usize> {
    match lead {
        0x00..=0x7F => Some(1),
        0xC2..=0xDF => Some(2),
        0xE0..=0xEF => Some(3),
        0xF0..=0xF4 => Some(4),
        _ => None,
    }
}

fn is_lower_hex(s: &str) -> bool {
    s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::{Info, SOURCE_DIGEST_HEX_LEN};

    fn sample() -> Info {
        Info {
            format_version: 1,
            generator: "glyphcull-compiler".to_string(),
            generator_version: "0.1.0".to_string(),
            source_digest: "ab".repeat(32),
            document_id: "cd".repeat(16),
            title: Some("A \"quoted\" title\nwith unicode: \u{4e2d}\u{6587}".to_string()),
            lang: Some("en".to_string()),
            chunk_count: 3,
            style_count: 2,
            content_count: 4,
            atlas_count: 1,
            image_count: 0,
        }
    }

    #[test]
    fn round_trip() {
        let info = sample();
        let bytes = info.encode();
        assert_eq!(Info::decode(&bytes).expect("decode"), info);
    }

    #[test]
    fn round_trip_no_optionals() {
        let mut info = sample();
        info.title = None;
        info.lang = None;
        let bytes = info.encode();
        assert_eq!(Info::decode(&bytes).expect("decode"), info);
    }

    #[test]
    fn sorted_keys_no_whitespace() {
        let bytes = sample().encode();
        let text = core::str::from_utf8(&bytes).expect("utf8");
        assert!(text.starts_with("{\"atlas_count\""));
        assert!(text.ends_with('}'));
        // No whitespace outside string literals.
        let mut in_string = false;
        let mut escaped = false;
        for &b in &bytes {
            if escaped {
                escaped = false;
                continue;
            }
            match b {
                b'\\' if in_string => escaped = true,
                b'"' => in_string = !in_string,
                b if b.is_ascii_whitespace() && !in_string => {
                    panic!("whitespace outside a string literal")
                }
                _ => {}
            }
        }
        // Keys are lexicographically sorted.
        let keys: Vec<&str> = text[1..text.len() - 1]
            .split(',')
            .filter_map(|pair| pair.split(':').next())
            .map(|k| &k[1..k.len() - 1])
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }

    #[test]
    fn rejects_trailing_data() {
        let mut bytes = sample().encode();
        bytes.push(b'x');
        assert!(Info::decode(&bytes).is_err());
    }

    #[test]
    fn rejects_unknown_keys() {
        let bytes = sample().encode();
        let mut text = String::from_utf8(bytes).expect("utf8");
        let pos = text.find("chunk_count").expect("find");
        text.replace_range(pos..pos + 1, "x");
        assert!(Info::decode(text.as_bytes()).is_err());
    }

    #[test]
    fn digest_shape_validation() {
        let mut info = sample();
        info.document_id = "Z".repeat(32); // uppercase: not lower hex
        assert!(Info::decode(&info.encode()).is_err());
        let mut info = sample();
        info.source_digest = "ab".repeat(10); // wrong length
        assert!(Info::decode(&info.encode()).is_err());
        let _ = SOURCE_DIGEST_HEX_LEN;
    }

    #[test]
    fn version_must_match() {
        let mut info = sample();
        info.format_version = 2;
        assert!(Info::decode(&info.encode()).is_err());
    }

    #[test]
    fn duplicate_keys_rejected() {
        let bytes = sample().encode();
        let mut text = String::from_utf8(bytes).expect("utf8");
        // Duplicate the first key: {"atlas_count":0,"atlas_count":0,...}
        let insert_at = text.find(',').expect("first comma");
        text.insert_str(insert_at, ",\"atlas_count\":0");
        assert!(Info::decode(text.as_bytes()).is_err());
    }
}
