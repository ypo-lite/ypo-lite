//! A minimal PNG writer: 8-bit RGBA, no interlacing, no filtering.
//!
//! Just enough to hand an icon to a UI toolkit that wants encoded image
//! bytes (`windows-reactor`'s `Image::source_data`). The pixel data goes in
//! "stored" (uncompressed) deflate blocks, so there is no compressor, no
//! dependency, and no COM/WIC to initialise; the price is a file the size of
//! the raw pixels, which for an icon (4 KB at 32×32, 256 KB at 256×256) held
//! in memory for a moment is nothing.

/// The 8-byte signature every PNG starts with.
const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// The most a stored deflate block holds.
const STORED_MAX: usize = 65_535;

/// `rgba` is `width * height * 4` bytes, top row first, straight (not
/// premultiplied) alpha. Returns an empty vector for a size that does not
/// match, rather than a corrupt file.
pub fn encode_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let row = width as usize * 4;
    if width == 0 || height == 0 || rgba.len() != row * height as usize {
        return Vec::new();
    }

    // Each scanline is a filter byte (0: none) and its pixels.
    let mut raw = Vec::with_capacity((row + 1) * height as usize);
    for line in rgba.chunks_exact(row) {
        raw.push(0);
        raw.extend_from_slice(line);
    }

    // zlib: header, stored blocks, Adler-32 of the raw data.
    let mut zlib = vec![0x78, 0x01];
    let mut blocks = raw.chunks(STORED_MAX).peekable();
    while let Some(block) = blocks.next() {
        let last = blocks.peek().is_none();
        zlib.push(u8::from(last));
        zlib.extend_from_slice(&(block.len() as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 bits, RGBA, deflate, adaptive filter, no interlace

    let mut png = Vec::with_capacity(SIGNATURE.len() + zlib.len() + 64);
    png.extend_from_slice(&SIGNATURE);
    chunk(&mut png, *b"IHDR", &header);
    chunk(&mut png, *b"IDAT", &zlib);
    chunk(&mut png, *b"IEND", &[]);
    png
}

fn chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(&kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checksums_match_their_published_test_vectors() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    /// Reads a file back with the chunk and deflate structure it promises:
    /// every CRC right, the stored blocks adding up to the scanlines, the
    /// Adler-32 matching them. Returns the raw (filtered) scanlines.
    fn read_back(png: &[u8]) -> (u32, u32, Vec<u8>) {
        assert_eq!(&png[..8], &SIGNATURE);
        let (mut at, mut size, mut idat) = (8, (0, 0), Vec::new());
        while at < png.len() {
            let length = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            let body = &png[at + 4..at + 8 + length];
            let crc =
                u32::from_be_bytes(png[at + 8 + length..at + 12 + length].try_into().unwrap());
            assert_eq!(crc32(body), crc, "CRC of {:?}", &body[..4]);
            match &body[..4] {
                b"IHDR" => {
                    size = (
                        u32::from_be_bytes(body[4..8].try_into().unwrap()),
                        u32::from_be_bytes(body[8..12].try_into().unwrap()),
                    );
                    assert_eq!(&body[12..17], &[8, 6, 0, 0, 0]);
                }
                b"IDAT" => idat.extend_from_slice(&body[4..]),
                _ => {}
            }
            at += 12 + length;
        }

        assert_eq!(&idat[..2], &[0x78, 0x01]);
        let (mut raw, mut at) = (Vec::new(), 2);
        loop {
            let last = idat[at] == 1;
            let len = u16::from_le_bytes([idat[at + 1], idat[at + 2]]) as usize;
            let complement = u16::from_le_bytes([idat[at + 3], idat[at + 4]]);
            assert_eq!(complement, !(len as u16));
            raw.extend_from_slice(&idat[at + 5..at + 5 + len]);
            at += 5 + len;
            if last {
                break;
            }
        }
        assert_eq!(
            u32::from_be_bytes(idat[at..at + 4].try_into().unwrap()),
            adler32(&raw)
        );
        assert_eq!(at + 4, idat.len());
        (size.0, size.1, raw)
    }

    #[test]
    fn a_small_image_round_trips_through_its_own_structure() {
        let rgba: Vec<u8> = (0..2 * 3 * 4).map(|n| n as u8).collect();
        let (width, height, raw) = read_back(&encode_rgba(2, 3, &rgba));
        assert_eq!((width, height), (2, 3));
        let expected: Vec<u8> = rgba
            .chunks(8)
            .flat_map(|line| std::iter::once(0).chain(line.iter().copied()))
            .collect();
        assert_eq!(raw, expected);
    }

    #[test]
    fn an_image_bigger_than_one_stored_block_is_split() {
        let rgba = vec![0x7F; 100 * 100 * 4];
        let (_, _, raw) = read_back(&encode_rgba(100, 100, &rgba));
        assert_eq!(raw.len(), 100 * (100 * 4 + 1));
    }

    #[test]
    fn a_size_that_does_not_match_gives_nothing() {
        assert!(encode_rgba(2, 2, &[0; 15]).is_empty());
        assert!(encode_rgba(0, 2, &[]).is_empty());
    }
}
