//! CONT section codec (SPEC.md §2.4): content payloads (text and image refs).

use crate::error::{Error, Result};
use crate::util::{Cursor, Writer};

/// Maximum payload count (SPEC.md §1.3).
pub const MAX_PAYLOAD_COUNT: u32 = 1 << 24;

/// Payload kinds (SPEC.md §2.4).
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum PayloadKind {
    /// UTF-8 text (NFC).
    TextUtf8 = 0,
    /// A `u32` image id into IMGS.
    ImageRef = 1,
}

impl PayloadKind {
    /// The wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Parse a wire value.
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::TextUtf8),
            1 => Some(Self::ImageRef),
            _ => None,
        }
    }
}

/// One content payload (`payloads[i].id == i`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload {
    /// The payload kind.
    pub kind: PayloadKind,
    /// Payload bytes (text: UTF-8; image ref: 4-byte LE image id).
    pub data: Vec<u8>,
}

/// The decoded CONT section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentSection {
    /// Payloads in dense id order.
    pub payloads: Vec<Payload>,
}

impl ContentSection {
    /// Encode to the CONT payload.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(self.payloads.len() as u32);
        for (i, payload) in self.payloads.iter().enumerate() {
            w.u32(i as u32);
            w.u8(payload.kind.to_u8());
            w.u8(0); // flags
            w.u16(0); // reserved
            w.u32(payload.data.len() as u32);
            w.bytes(&payload.data);
        }
        w.into_bytes()
    }

    /// Decode and structurally validate the CONT payload.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(bytes);
        let payload_count = c.u32("payload count")?;
        if payload_count > MAX_PAYLOAD_COUNT {
            return Err(Error::LimitExceeded {
                what: "payload count",
                value: u64::from(payload_count),
                limit: u64::from(MAX_PAYLOAD_COUNT),
            });
        }
        let mut payloads = Vec::with_capacity(payload_count as usize);
        for i in 0..payload_count {
            let id = c.u32("payload id")?;
            if id != i {
                return Err(Error::UnknownValue {
                    what: "payload id order",
                    value: u64::from(id),
                });
            }
            let kind = PayloadKind::from_u8(c.u8("payload kind")?).ok_or(Error::UnknownValue {
                what: "payload kind",
                value: 0,
            })?;
            let flags = c.u8("payload flags")?;
            let reserved = c.u16("payload reserved")?;
            let data_len = c.u32("payload data len")?;
            if flags != 0 || reserved != 0 {
                return Err(Error::ReservedBitsSet);
            }
            let data = c.take(data_len as usize, "payload data")?.to_vec();
            match kind {
                PayloadKind::TextUtf8 => {
                    core::str::from_utf8(&data).map_err(|_| Error::InvalidUtf8)?;
                }
                PayloadKind::ImageRef => {
                    if data.len() != 4 {
                        return Err(Error::UnknownValue {
                            what: "image_ref payload length",
                            value: data.len() as u64,
                        });
                    }
                }
            }
            payloads.push(Payload { kind, data });
        }
        c.finish("CONT payload")?;
        Ok(Self { payloads })
    }
}

#[cfg(test)]
mod tests {
    use super::{ContentSection, Payload, PayloadKind};

    fn sample() -> ContentSection {
        ContentSection {
            payloads: vec![
                Payload {
                    kind: PayloadKind::TextUtf8,
                    data: "Hello, world! \u{4e2d}\u{6587}".as_bytes().to_vec(),
                },
                Payload {
                    kind: PayloadKind::ImageRef,
                    data: 7_u32.to_le_bytes().to_vec(),
                },
            ],
        }
    }

    #[test]
    fn round_trip() {
        let section = sample();
        let bytes = section.encode();
        assert_eq!(ContentSection::decode(&bytes).expect("decode"), section);
    }

    #[test]
    fn invalid_utf8_rejected() {
        let section = sample();
        let bytes = section.encode();
        let mut corrupted = bytes;
        // Corrupt a byte inside the text payload (offset 12 header + text region).
        corrupted[12 + 6] = 0xFF;
        assert!(ContentSection::decode(&corrupted).is_err());
    }

    #[test]
    fn bad_image_ref_len_rejected() {
        let mut section = sample();
        section.payloads[1].data = vec![1, 2, 3];
        assert!(ContentSection::decode(&section.encode()).is_err());
    }

    #[test]
    fn id_order_enforced() {
        let section = sample();
        let bytes = section.encode();
        let mut corrupted = bytes;
        corrupted[4] = 9; // first payload id
        assert!(ContentSection::decode(&corrupted).is_err());
    }
}
