//! Minimal gzip (RFC 1952) and DEFLATE (RFC 1951) decoder, written from the
//! RFCs, used to read Golly's `.mc.gz` metapixel demos without a
//! third-party dependency. It favours clarity over speed (bit-by-bit
//! canonical Huffman decoding): the demos decompress in milliseconds.
//!
//! Every member's CRC-32 and length are verified, and the output size is
//! capped so that a malicious stream cannot exhaust memory.

use crate::error::MetapixelError;

/// Default cap on decompressed output (256 MiB).
pub const DEFAULT_MAX_OUTPUT: usize = 256 << 20;

fn fail(message: impl Into<String>) -> MetapixelError {
    MetapixelError::Gzip(message.into())
}

/// Whether `bytes` start with the gzip magic number.
pub fn is_gzip(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x1F, 0x8B])
}

/// Decompresses a gzip file (one or more members) into at most
/// `max_output` bytes.
///
/// # Errors
/// [`MetapixelError::Gzip`] for a bad header, unsupported compression
/// method, corrupt DEFLATE data, a CRC-32 or length mismatch, truncated
/// input, or output larger than `max_output`.
pub fn decompress(bytes: &[u8], max_output: usize) -> Result<Vec<u8>, MetapixelError> {
    let mut out = Vec::new();
    let mut pos = 0;
    loop {
        let member_start = out.len();
        pos = skip_header(bytes, pos)?;
        let mut reader = BitReader { data: bytes, pos, bit: 0, bits_in: 0 };
        inflate(&mut reader, &mut out, max_output)?;
        pos = reader.byte_position();
        let trailer = bytes.get(pos..pos + 8).ok_or_else(|| fail("truncated trailer (CRC-32 and size missing)"))?;
        let crc = u32::from_le_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
        let size = u32::from_le_bytes([trailer[4], trailer[5], trailer[6], trailer[7]]);
        let member = &out[member_start..];
        if crc32(member) != crc {
            return Err(fail("CRC-32 mismatch: the data is corrupt"));
        }
        if member.len() as u32 != size {
            return Err(fail(format!("length mismatch: trailer says {size} bytes, decoded {}", member.len())));
        }
        pos += 8;
        if pos >= bytes.len() || !is_gzip(&bytes[pos..]) {
            return Ok(out);
        }
    }
}

/// Parses a member header starting at `pos`; returns the position of the
/// DEFLATE data.
fn skip_header(bytes: &[u8], pos: usize) -> Result<usize, MetapixelError> {
    let header = bytes.get(pos..pos + 10).ok_or_else(|| fail("truncated header"))?;
    if header[0..2] != [0x1F, 0x8B] {
        return Err(fail("not a gzip file (bad magic number)"));
    }
    if header[2] != 8 {
        return Err(fail(format!("unsupported compression method {} (only DEFLATE = 8)", header[2])));
    }
    let flags = header[3];
    if flags & 0xE0 != 0 {
        return Err(fail("reserved header flags are set"));
    }
    let mut p = pos + 10;
    if flags & 0x04 != 0 {
        let len = bytes.get(p..p + 2).ok_or_else(|| fail("truncated extra field"))?;
        p += 2 + usize::from(u16::from_le_bytes([len[0], len[1]]));
    }
    for flag in [0x08, 0x10] {
        if flags & flag != 0 {
            let rest = bytes.get(p..).ok_or_else(|| fail("truncated header"))?;
            let end = rest.iter().position(|&b| b == 0).ok_or_else(|| fail("unterminated file name or comment"))?;
            p += end + 1;
        }
    }
    if flags & 0x02 != 0 {
        p += 2;
    }
    if p > bytes.len() {
        return Err(fail("truncated header"));
    }
    Ok(p)
}

/// CRC-32 (IEEE 802.3, reflected polynomial 0xEDB88320).
pub fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *slot = c;
    }
    !data.iter().fold(!0u32, |c, &b| table[((c ^ u32::from(b)) & 0xFF) as usize] ^ (c >> 8))
}

/// LSB-first bit reader.
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u32,
    bits_in: u32,
}

impl BitReader<'_> {
    fn bits(&mut self, count: u32) -> Result<u32, MetapixelError> {
        let mut value = 0u32;
        for i in 0..count {
            if self.bits_in == 0 {
                self.bit = u32::from(*self.data.get(self.pos).ok_or_else(|| fail("unexpected end of DEFLATE data"))?);
                self.pos += 1;
                self.bits_in = 8;
            }
            value |= (self.bit & 1) << i;
            self.bit >>= 1;
            self.bits_in -= 1;
        }
        Ok(value)
    }

    fn align_to_byte(&mut self) {
        self.bits_in = 0;
    }

    fn byte_position(&self) -> usize {
        self.pos
    }
}

/// Canonical Huffman code: number of codes per length and the symbols in
/// code order.
struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Self, MetapixelError> {
        let mut counts = [0u16; 16];
        for &len in lengths {
            counts[usize::from(len)] += 1;
        }
        counts[0] = 0;
        // Reject over-subscribed codes (incomplete codes are allowed by the RFC).
        let mut left = 1i32;
        for &count in &counts[1..] {
            left = left * 2 - i32::from(count);
            if left < 0 {
                return Err(fail("over-subscribed Huffman code"));
            }
        }
        let mut offsets = [0u16; 16];
        for len in 1..15 {
            offsets[len + 1] = offsets[len] + counts[len];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (symbol, &len) in lengths.iter().enumerate() {
            if len != 0 {
                symbols[usize::from(offsets[usize::from(len)])] = symbol as u16;
                offsets[usize::from(len)] += 1;
            }
        }
        Ok(Self { counts, symbols })
    }

    fn decode(&self, reader: &mut BitReader<'_>) -> Result<u16, MetapixelError> {
        // `first` is the first code of the current length, `index` the
        // position of its symbol.
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= reader.bits(1)? as i32;
            let count = i32::from(self.counts[len]);
            if code - first < count {
                return Ok(self.symbols[(index + code - first) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(fail("invalid Huffman code"))
    }
}

const LENGTH_BASE: [u16; 29] =
    [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LENGTH_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
    8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] =
    [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
/// Order in which code-length code lengths are stored (RFC 1951 §3.2.7).
const CODE_LENGTH_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// Inflates one DEFLATE stream, appending to `out`.
fn inflate(reader: &mut BitReader<'_>, out: &mut Vec<u8>, max_output: usize) -> Result<(), MetapixelError> {
    loop {
        let last = reader.bits(1)? == 1;
        match reader.bits(2)? {
            0 => stored_block(reader, out, max_output)?,
            1 => {
                let (lit, dist) = fixed_codes()?;
                compressed_block(reader, out, &lit, &dist, max_output)?;
            }
            2 => {
                let (lit, dist) = dynamic_codes(reader)?;
                compressed_block(reader, out, &lit, &dist, max_output)?;
            }
            _ => return Err(fail("invalid block type 3")),
        }
        if last {
            return Ok(());
        }
    }
}

fn check_room(out: &[u8], extra: usize, max_output: usize) -> Result<(), MetapixelError> {
    if out.len().saturating_add(extra) > max_output {
        return Err(fail(format!("decompressed data exceeds the limit of {max_output} bytes")));
    }
    Ok(())
}

fn stored_block(reader: &mut BitReader<'_>, out: &mut Vec<u8>, max_output: usize) -> Result<(), MetapixelError> {
    reader.align_to_byte();
    let p = reader.pos;
    let header = reader.data.get(p..p + 4).ok_or_else(|| fail("truncated stored block"))?;
    let len = u16::from_le_bytes([header[0], header[1]]);
    let nlen = u16::from_le_bytes([header[2], header[3]]);
    if len != !nlen {
        return Err(fail("stored block length check failed"));
    }
    let data = reader.data.get(p + 4..p + 4 + usize::from(len)).ok_or_else(|| fail("truncated stored block"))?;
    check_room(out, data.len(), max_output)?;
    out.extend_from_slice(data);
    reader.pos = p + 4 + usize::from(len);
    Ok(())
}

fn fixed_codes() -> Result<(Huffman, Huffman), MetapixelError> {
    let mut lengths = [0u8; 288];
    for (symbol, len) in lengths.iter_mut().enumerate() {
        *len = match symbol {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    Ok((Huffman::new(&lengths)?, Huffman::new(&[5u8; 30])?))
}

fn dynamic_codes(reader: &mut BitReader<'_>) -> Result<(Huffman, Huffman), MetapixelError> {
    let literals = reader.bits(5)? as usize + 257;
    let distances = reader.bits(5)? as usize + 1;
    let code_lengths = reader.bits(4)? as usize + 4;
    if literals > 286 || distances > 30 {
        return Err(fail("too many length or distance codes"));
    }
    let mut cl_lengths = [0u8; 19];
    for &slot in &CODE_LENGTH_ORDER[..code_lengths] {
        cl_lengths[slot] = reader.bits(3)? as u8;
    }
    let cl_code = Huffman::new(&cl_lengths)?;
    let mut lengths = Vec::with_capacity(literals + distances);
    while lengths.len() < literals + distances {
        let symbol = cl_code.decode(reader)?;
        let (value, repeat) = match symbol {
            0..=15 => (symbol as u8, 1),
            16 => {
                let previous = *lengths.last().ok_or_else(|| fail("repeat code with no previous length"))?;
                (previous, 3 + reader.bits(2)? as usize)
            }
            17 => (0, 3 + reader.bits(3)? as usize),
            _ => (0, 11 + reader.bits(7)? as usize),
        };
        if lengths.len() + repeat > literals + distances {
            return Err(fail("code lengths overflow the declared counts"));
        }
        lengths.extend(std::iter::repeat_n(value, repeat));
    }
    if lengths[256] == 0 {
        return Err(fail("the end-of-block code has no length"));
    }
    Ok((Huffman::new(&lengths[..literals])?, Huffman::new(&lengths[literals..])?))
}

fn compressed_block(
    reader: &mut BitReader<'_>,
    out: &mut Vec<u8>,
    lit: &Huffman,
    dist: &Huffman,
    max_output: usize,
) -> Result<(), MetapixelError> {
    loop {
        let symbol = lit.decode(reader)?;
        match symbol {
            0..=255 => {
                check_room(out, 1, max_output)?;
                out.push(symbol as u8);
            }
            256 => return Ok(()),
            257..=285 => {
                let i = usize::from(symbol - 257);
                let length = usize::from(LENGTH_BASE[i]) + reader.bits(u32::from(LENGTH_EXTRA[i]))? as usize;
                let d = usize::from(dist.decode(reader)?);
                if d >= 30 {
                    return Err(fail("invalid distance code"));
                }
                let distance = usize::from(DIST_BASE[d]) + reader.bits(u32::from(DIST_EXTRA[d]))? as usize;
                if distance > out.len() {
                    return Err(fail("distance reaches before the start of the output"));
                }
                check_room(out, length, max_output)?;
                let start = out.len() - distance;
                for k in 0..length {
                    let byte = out[start + k];
                    out.push(byte);
                }
            }
            _ => return Err(fail("invalid literal/length code")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `printf 'hello hello hello hello\n' | gzip -9n` (a fixed-Huffman
    /// block with back references).
    const HELLO_GZ: [u8; 29] = [
        0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x03, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40,
        0x27, 0xb9, 0x00, 0x00, 0x88, 0x59, 0x0b, 0x18, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn crc32_reference_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn stored_block_round_trip() {
        let data = b"metapixel";
        let mut gz = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
        gz.push(1); // BFINAL = 1, BTYPE = 00
        gz.extend_from_slice(&(data.len() as u16).to_le_bytes());
        gz.extend_from_slice(&(!(data.len() as u16)).to_le_bytes());
        gz.extend_from_slice(data);
        gz.extend_from_slice(&crc32(data).to_le_bytes());
        gz.extend_from_slice(&(data.len() as u32).to_le_bytes());
        assert_eq!(decompress(&gz, 1024).unwrap(), data);
        assert!(decompress(&gz, 3).is_err(), "output cap");
        let mut corrupt = gz.clone();
        corrupt[16] ^= 1;
        assert!(decompress(&corrupt, 1024).is_err(), "CRC mismatch");
        assert!(decompress(&gz[..gz.len() - 3], 1024).is_err(), "truncated");
        assert!(decompress(b"plain text", 1024).is_err());
    }

    #[test]
    fn fixed_huffman_member_with_back_references() {
        assert!(is_gzip(&HELLO_GZ));
        assert_eq!(decompress(&HELLO_GZ, 1024).unwrap(), b"hello hello hello hello\n");
        let mut twice = HELLO_GZ.to_vec();
        twice.extend_from_slice(&HELLO_GZ);
        assert_eq!(decompress(&twice, 1024).unwrap().len(), 48, "concatenated members");
        let mut corrupt = HELLO_GZ;
        corrupt[21] ^= 0x40;
        assert!(decompress(&corrupt, 1024).is_err(), "CRC-32 is verified");
    }
}
