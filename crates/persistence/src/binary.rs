//! The `.elsnap` binary container.
//!
//! ```text
//! offset  size  field
//!      0     8  magic  89 45 4C 53 4E 41 50 0A  ("\x89ELSNAP\n")
//!      8     2  container version (u16, little endian) = 1
//!     10     2  flags (u16, must be 0)
//!     12     4  header length H (u32 LE)
//!     16     8  payload length P (u64 LE)
//!     24     4  CRC-32 (IEEE) of the header and payload bytes (u32 LE)
//!     28     H  envelope header, postcard
//!   28+H     P  payload, postcard
//! ```
//!
//! The magic number starts with a non-ASCII byte and ends with a line feed,
//! so files mangled by text-mode transfers are detected. Lengths are
//! validated against the actual size (truncation and trailing data are
//! errors) and the checksum detects corruption before decoding.

use std::io::Read;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::envelope::EnvelopeHeader;
use crate::error::PersistenceError;

/// Decodes a postcard value that must occupy all of `bytes`.
fn decode_exact<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    let (value, rest) = postcard::take_from_bytes(bytes).map_err(|e| e.to_string())?;
    if rest.is_empty() { Ok(value) } else { Err(format!("{} unexpected byte(s) after the data", rest.len())) }
}

/// Magic bytes at the start of every `.elsnap` file.
pub const MAGIC: [u8; 8] = *b"\x89ELSNAP\n";

/// Version of the container layout written by this build.
pub const CONTAINER_VERSION: u16 = 1;

/// Size of the fixed prefix before the header bytes.
pub const PREFIX_LEN: usize = 28;

/// Largest accepted header (the header holds a few short strings).
pub const MAX_HEADER_LEN: u32 = 64 * 1024;

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = crc_table();

/// Incremental CRC-32 (IEEE 802.3, reflected polynomial `0xEDB88320`).
#[derive(Clone, Copy, Debug)]
pub struct Crc32(u32);

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc32 {
    /// Fresh checksum state.
    pub fn new() -> Self {
        Self(!0)
    }

    /// Feeds bytes.
    pub fn update(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = CRC_TABLE[((self.0 ^ u32::from(b)) & 0xFF) as usize] ^ (self.0 >> 8);
        }
    }

    /// Final checksum.
    pub fn finish(self) -> u32 {
        !self.0
    }

    /// CRC-32 of a byte slice.
    pub fn of(bytes: &[u8]) -> u32 {
        let mut crc = Self::new();
        crc.update(bytes);
        crc.finish()
    }
}

/// Encodes a header and payload into an `.elsnap` byte image.
pub fn encode<T: Serialize>(header: &EnvelopeHeader, payload: &T) -> Result<Vec<u8>, PersistenceError> {
    let encode_err =
        |e: postcard::Error| PersistenceError::Encode { format: crate::Encoding::Binary, message: e.to_string() };
    let header_bytes = postcard::to_stdvec(header).map_err(encode_err)?;
    let payload_bytes = postcard::to_stdvec(payload).map_err(encode_err)?;
    let header_len = u32::try_from(header_bytes.len()).ok().filter(|&len| len <= MAX_HEADER_LEN).ok_or_else(|| {
        PersistenceError::TooLarge {
            what: "file header",
            size: header_bytes.len() as u64,
            limit: MAX_HEADER_LEN.into(),
        }
    })?;
    let mut crc = Crc32::new();
    crc.update(&header_bytes);
    crc.update(&payload_bytes);

    let mut out = Vec::with_capacity(PREFIX_LEN + header_bytes.len() + payload_bytes.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&CONTAINER_VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&header_len.to_le_bytes());
    out.extend_from_slice(&(payload_bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(&crc.finish().to_le_bytes());
    out.extend_from_slice(&header_bytes);
    out.extend_from_slice(&payload_bytes);
    Ok(out)
}

/// Fields of the fixed prefix.
struct Prefix {
    header_len: u32,
    payload_len: u64,
    crc: u32,
}

/// Validates the magic number, container version, flags and header length
/// of the first bytes of a file (`bytes` may be shorter than the prefix).
fn parse_prefix(bytes: &[u8]) -> Result<Prefix, PersistenceError> {
    if bytes.len() < MAGIC.len() || bytes[..MAGIC.len()] != MAGIC {
        return Err(PersistenceError::NotEmergenceLab {
            reason: "the data does not start with the .elsnap signature".to_string(),
        });
    }
    if bytes.len() < PREFIX_LEN {
        return Err(PersistenceError::CorruptBinary(format!(
            "the file is truncated ({} bytes, the fixed header alone needs {PREFIX_LEN})",
            bytes.len()
        )));
    }
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    let version = u16_at(8);
    if version > CONTAINER_VERSION {
        return Err(PersistenceError::CorruptBinary(format!(
            "container version {version} is newer than this build supports ({CONTAINER_VERSION}); update Emergence Lab"
        )));
    }
    if version == 0 {
        return Err(PersistenceError::CorruptBinary("container version 0 is invalid".to_string()));
    }
    let flags = u16_at(10);
    if flags != 0 {
        return Err(PersistenceError::CorruptBinary(format!("unknown flags {flags:#06x}")));
    }
    let header_len = u32_at(12);
    if header_len > MAX_HEADER_LEN {
        return Err(PersistenceError::CorruptBinary(format!("header length {header_len} is implausibly large")));
    }
    let mut payload_len = [0u8; 8];
    payload_len.copy_from_slice(&bytes[16..24]);
    Ok(Prefix { header_len, payload_len: u64::from_le_bytes(payload_len), crc: u32_at(24) })
}

fn decode_header(bytes: &[u8]) -> Result<EnvelopeHeader, PersistenceError> {
    decode_exact(bytes).map_err(|e| PersistenceError::CorruptBinary(format!("unreadable file header: {e}")))
}

/// Reads only the fixed prefix and the envelope header from the start of an
/// `.elsnap` stream, without reading the payload (so the checksum is not
/// verified). Used to list saved files quickly.
pub fn read_header<R: Read>(reader: R) -> Result<EnvelopeHeader, PersistenceError> {
    let mut reader = reader;
    let mut prefix = Vec::with_capacity(PREFIX_LEN);
    (&mut reader)
        .take(PREFIX_LEN as u64)
        .read_to_end(&mut prefix)
        .map_err(|e| PersistenceError::io("read the file", e))?;
    let parsed = parse_prefix(&prefix)?;
    let mut header = Vec::with_capacity(parsed.header_len as usize);
    reader
        .take(u64::from(parsed.header_len))
        .read_to_end(&mut header)
        .map_err(|e| PersistenceError::io("read the file", e))?;
    if header.len() < parsed.header_len as usize {
        return Err(PersistenceError::CorruptBinary("the file is truncated inside its header".to_string()));
    }
    decode_header(&header)
}

/// A validated `.elsnap` image whose payload has not been decoded yet.
#[derive(Debug)]
pub struct BinaryImage<'a> {
    /// Decoded envelope header.
    pub header: EnvelopeHeader,
    payload: &'a [u8],
}

impl<'a> BinaryImage<'a> {
    /// Checks magic, container version, lengths and checksum, and decodes the
    /// header.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, PersistenceError> {
        let prefix = parse_prefix(bytes)?;
        let body = &bytes[PREFIX_LEN..];
        let declared = u64::from(prefix.header_len).checked_add(prefix.payload_len);
        if declared != Some(body.len() as u64) {
            let what = if declared.is_none_or(|d| d > body.len() as u64) {
                "truncated"
            } else {
                "followed by unexpected data"
            };
            return Err(PersistenceError::CorruptBinary(format!(
                "the file is {what}: the header declares {} bytes of content but {} are present",
                u64::from(prefix.header_len).saturating_add(prefix.payload_len),
                body.len()
            )));
        }
        let actual_crc = Crc32::of(body);
        if actual_crc != prefix.crc {
            return Err(PersistenceError::CorruptBinary(format!(
                "checksum mismatch (stored {:#010x}, computed {actual_crc:#010x})",
                prefix.crc
            )));
        }
        let (header_bytes, payload) = body.split_at(prefix.header_len as usize);
        Ok(Self { header: decode_header(header_bytes)?, payload })
    }

    /// Size of the encoded payload in bytes.
    pub fn payload_len(&self) -> usize {
        self.payload.len()
    }

    /// Decodes the payload as `T`. The payload must be consumed entirely: a
    /// type that reads fewer bytes than were written is a schema mismatch,
    /// reported as [`PersistenceError::InvalidPayload`].
    pub fn decode_payload<T: DeserializeOwned>(&self) -> Result<T, PersistenceError> {
        decode_exact(self.payload).map_err(|message| PersistenceError::InvalidPayload {
            kind: self.header.kind,
            version: self.header.schema_version,
            message,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::{FORMAT_TAG, FileKind};

    fn header() -> EnvelopeHeader {
        EnvelopeHeader {
            format: FORMAT_TAG.to_string(),
            kind: FileKind::Snapshot,
            schema_version: 1,
            app_version: "0.1.0".to_string(),
            model_id: Some("life.conway".to_string()),
            created: "now".to_string(),
        }
    }

    #[test]
    fn crc32_check_value() {
        // Standard check value of CRC-32/ISO-HDLC.
        assert_eq!(Crc32::of(b"123456789"), 0xCBF4_3926);
        assert_eq!(Crc32::of(b""), 0);
    }

    #[test]
    fn round_trip() {
        let payload = vec![1u32, 2, 3, 400_000];
        let bytes = encode(&header(), &payload).unwrap();
        assert_eq!(&bytes[..8], &MAGIC);
        let image = BinaryImage::parse(&bytes).unwrap();
        assert_eq!(image.header, header());
        assert_eq!(image.decode_payload::<Vec<u32>>().unwrap(), payload);
    }

    #[test]
    fn payload_must_be_consumed_entirely() {
        // A (u8, u32, String) payload must not decode as a bare u8 just
        // because its first byte happens to be a valid u8.
        let bytes = encode(&header(), &(5u8, 7u32, String::from("hello"))).unwrap();
        let image = BinaryImage::parse(&bytes).unwrap();
        let err = image.decode_payload::<u8>().unwrap_err();
        assert!(
            matches!(&err, PersistenceError::InvalidPayload { kind: FileKind::Snapshot, version: 1, message } if message.contains("unexpected")),
            "{err}"
        );
        assert_eq!(image.decode_payload::<(u8, u32, String)>().unwrap(), (5, 7, "hello".to_string()));
    }

    #[test]
    fn detects_damage() {
        let bytes = encode(&header(), &vec![7u8; 100]).unwrap();
        // Truncation.
        assert!(
            matches!(BinaryImage::parse(&bytes[..bytes.len() - 1]), Err(PersistenceError::CorruptBinary(m)) if m.contains("truncated"))
        );
        assert!(matches!(BinaryImage::parse(&bytes[..20]), Err(PersistenceError::CorruptBinary(_))));
        // Trailing garbage.
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(
            matches!(BinaryImage::parse(&longer), Err(PersistenceError::CorruptBinary(m)) if m.contains("unexpected data"))
        );
        // Flipped payload bit.
        let mut flipped = bytes.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 1;
        assert!(
            matches!(BinaryImage::parse(&flipped), Err(PersistenceError::CorruptBinary(m)) if m.contains("checksum"))
        );
        // Wrong magic / text file.
        assert!(matches!(
            BinaryImage::parse(b"(format: \"emergence-lab\")"),
            Err(PersistenceError::NotEmergenceLab { .. })
        ));
        // Newer container.
        let mut newer = bytes.clone();
        newer[8] = 2;
        assert!(matches!(BinaryImage::parse(&newer), Err(PersistenceError::CorruptBinary(m)) if m.contains("newer")));
        // Unknown flags.
        let mut flagged = bytes;
        flagged[10] = 1;
        assert!(BinaryImage::parse(&flagged).is_err());
    }

    #[test]
    fn header_can_be_read_without_payload() {
        let bytes = encode(&header(), &vec![9u8; 10_000]).unwrap();
        assert_eq!(read_header(&bytes[..]).unwrap(), header());
        // Only the prefix and header are needed.
        let header_end = PREFIX_LEN + u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
        assert_eq!(read_header(&bytes[..header_end]).unwrap(), header());
        assert!(matches!(read_header(&bytes[..header_end - 1]), Err(PersistenceError::CorruptBinary(_))));
        assert!(matches!(read_header(&bytes[..5]), Err(PersistenceError::NotEmergenceLab { .. })));
        assert!(matches!(read_header(&bytes[..20]), Err(PersistenceError::CorruptBinary(_))));
    }
}
