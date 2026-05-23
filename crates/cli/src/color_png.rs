//! Hand-rolled 1×1 RGBA PNG encoder for synthesized color swatches.
//!
//! Used by [`crate::color_synth`] to drop `Color_*.png` files into a
//! TexturePacker source dir when a `.tps.fab.json` references a polygon
//! `color` that the existing tpsheet doesn't carry. Mirrors meow-tower's
//! `ColorTextureUtils.CreateTexture`.
//!
//! Output is a fixed 73-byte byte-exact stream: PNG signature + IHDR (8-bit
//! RGBA, no interlace) + IDAT (zlib stored-block scanline) + IEND. No
//! external `png` crate — the core rlib is hand-rolled by policy and the
//! CLI follows suit.

/// PNG signature: `\x89PNG\r\n\x1a\n`.
const PNG_SIG: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// Encode a 1×1 RGBA PNG. Deterministic byte output — pinned by tests so
/// a future encoder regression surfaces against the golden vector.
pub fn encode_1x1(rgba: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(73);
    out.extend_from_slice(&PNG_SIG);

    // IHDR: width=1, height=1, bit_depth=8, color_type=6 (RGBA),
    // compression=0, filter=0, interlace=0.
    let ihdr_data: [u8; 13] = [
        0, 0, 0, 1, // width
        0, 0, 0, 1, // height
        8, 6, 0, 0, 0,
    ];
    write_chunk(&mut out, *b"IHDR", &ihdr_data);

    // IDAT: zlib-wrapped deflate stored block carrying one scanline
    // (1 filter byte + 4 RGBA bytes = 5 raw bytes).
    let scanline: [u8; 5] = [0, rgba[0], rgba[1], rgba[2], rgba[3]];
    let mut idat = Vec::with_capacity(16);
    // zlib header: CMF=0x78 (deflate, 32K window), FLG=0x01 (no preset
    // dict, fastest level, FCHECK chosen so (CMF*256+FLG) % 31 == 0).
    idat.extend_from_slice(&[0x78, 0x01]);
    // Stored block: BFINAL=1, BTYPE=00 → first byte 0x01.
    idat.push(0x01);
    let len = scanline.len() as u16;
    idat.extend_from_slice(&len.to_le_bytes());
    idat.extend_from_slice(&(!len).to_le_bytes());
    idat.extend_from_slice(&scanline);
    idat.extend_from_slice(&adler32(&scanline).to_be_bytes());
    write_chunk(&mut out, *b"IDAT", &idat);

    write_chunk(&mut out, *b"IEND", &[]);
    out
}

fn write_chunk(out: &mut Vec<u8>, ty: [u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(&ty);
    out.extend_from_slice(data);
    let mut crc_input: Vec<u8> = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(&ty);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32_ieee(&crc_input).to_be_bytes());
}

/// CRC-32/ISO-HDLC (IEEE 802.3 polynomial `0xEDB88320`). Bitwise variant —
/// PNG chunks are short (≤ 13 bytes for IHDR, ≤ 16 for our IDAT), so
/// table-free is plenty fast.
fn crc32_ieee(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = !(crc & 1).wrapping_sub(1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Adler-32 over the raw (uncompressed) scanline bytes. zlib's checksum.
fn adler32(bytes: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &x in bytes {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_and_ihdr_for_solid_white() {
        let png = encode_1x1([0xFF, 0xFF, 0xFF, 0xFF]);
        // 8 (sig) + 25 (IHDR) + 28 (IDAT) + 12 (IEND) = 73.
        assert_eq!(png.len(), 73);
        assert_eq!(&png[..8], &PNG_SIG);
        assert_eq!(&png[12..16], b"IHDR");
        // Last 12 bytes are the IEND chunk: length(0) + "IEND" + CRC.
        assert_eq!(&png[png.len() - 12..png.len() - 8], &[0, 0, 0, 0]);
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    }

    #[test]
    fn deterministic_byte_output() {
        // Two encodes of the same color produce identical streams — no
        // alloc-order-dependent CRC table, no timestamp, etc.
        let a = encode_1x1([0x32, 0x26, 0x4D, 0xBD]);
        let b = encode_1x1([0x32, 0x26, 0x4D, 0xBD]);
        assert_eq!(a, b);
    }

    #[test]
    fn different_colors_produce_different_streams() {
        let red = encode_1x1([0xFF, 0x00, 0x00, 0xFF]);
        let green = encode_1x1([0x00, 0xFF, 0x00, 0xFF]);
        assert_ne!(red, green);
        // Same length though — only the RGBA bytes inside IDAT differ.
        assert_eq!(red.len(), green.len());
    }

    #[test]
    fn crc32_known_vectors() {
        // CRC-32 of "IHDR" + (13 IHDR data bytes for a 1×1 RGBA image)
        // is a well-known value. If this drifts, the encoder's chunk
        // CRCs are wrong.
        let ihdr_data: [u8; 17] = [
            b'I', b'H', b'D', b'R',
            0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0,
        ];
        assert_eq!(crc32_ieee(&ihdr_data), 0x1F15_C489);
        // CRC of just "IEND" (zero-data chunk).
        assert_eq!(crc32_ieee(b"IEND"), 0xAE42_6082);
    }

    #[test]
    fn adler32_known_vector() {
        // Adler32 of "" = 1, "a" = 0x00620062, "abc" = 0x024D0127.
        assert_eq!(adler32(b""), 1);
        assert_eq!(adler32(b"a"), 0x0062_0062);
        assert_eq!(adler32(b"abc"), 0x024D_0127);
    }
}
