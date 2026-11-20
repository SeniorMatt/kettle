//! Chunk and manifest decompression: Steam's VZip (LZMA, "VZa"), the newer
//! zstd wrapper ("VSZa") and plain PKZip (deflate or stored).

use anyhow::{Context, Result, bail, ensure};
use std::io::Read;

/// Inflate Steam chunk payload (already decrypted) whose real size is known.
pub fn decompress(data: &[u8], expected: usize) -> Result<Vec<u8>> {
    let out = if data.starts_with(b"VSZa") {
        zstd_wrapped(data, expected)?
    } else if data.starts_with(b"VZa") {
        vzip(data, expected)?
    } else if data.starts_with(b"PK\x03\x04") {
        unzip_first(data)?
    } else {
        bail!("unknown chunk compression (starts with {:02x?})", &data[..data.len().min(4)]);
    };
    ensure!(out.len() == expected, "decompressed to {} bytes, manifest says {expected}", out.len());
    Ok(out)
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// "VZa" + 4 bytes + 5 LZMA property bytes + raw LZMA + crc32 + size + "zv".
fn vzip(data: &[u8], expected: usize) -> Result<Vec<u8>> {
    const HEAD: usize = 7;
    const FOOT: usize = 10;
    ensure!(data.len() > HEAD + 5 + FOOT && data.ends_with(b"zv"), "truncated VZip");
    let footer = &data[data.len() - FOOT..];
    let (crc, size) = (le32(footer), le32(&footer[4..]) as usize);
    ensure!(size == expected, "VZip size {size} != {expected}");
    let mut src = &data[HEAD..data.len() - FOOT];
    let mut out = Vec::with_capacity(size);
    // The stream has no size field in its header, so hand the size to the decoder.
    let opts = lzma_rs::decompress::Options {
        unpacked_size: lzma_rs::decompress::UnpackedSize::UseProvided(Some(size as u64)),
        ..Default::default()
    };
    lzma_rs::lzma_decompress_with_options(&mut src, &mut out, &opts).map_err(|e| anyhow::anyhow!("LZMA: {e}"))?;
    ensure!(crc32fast::hash(&out) == crc, "VZip CRC mismatch");
    Ok(out)
}

/// "VSZa" + crc32 of the result + one zstd frame + a footer (crc, size, padding, "zsv").
/// The footer's exact layout varies, so only the header CRC and the frame itself are used.
fn zstd_wrapped(data: &[u8], expected: usize) -> Result<Vec<u8>> {
    ensure!(data.len() > 8 + 12 && data.ends_with(b"zsv"), "truncated VSZa");
    let crc = le32(&data[4..]);
    let mut dec = ruzstd::decoding::StreamingDecoder::new(&data[8..]).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut out = Vec::with_capacity(expected);
    dec.read_to_end(&mut out).context("zstd")?;
    ensure!(crc32fast::hash(&out) == crc, "VSZa CRC mismatch");
    Ok(out)
}

/// Contents of the first entry of a zip archive. Uses the central directory
/// because streamed zips leave sizes zero in the local header.
pub fn unzip_first(zip: &[u8]) -> Result<Vec<u8>> {
    let eocd = zip.windows(4).rposition(|w| w == b"PK\x05\x06").context("zip: no end record")?;
    ensure!(zip.len() >= eocd + 22, "zip: short end record");
    let cd = le32(&zip[eocd + 16..]) as usize;
    ensure!(zip.len() >= cd + 46 && &zip[cd..cd + 4] == b"PK\x01\x02", "zip: bad central directory");
    let method = u16::from_le_bytes([zip[cd + 10], zip[cd + 11]]);
    let (csize, usize_) = (le32(&zip[cd + 20..]) as usize, le32(&zip[cd + 24..]) as usize);
    let local = le32(&zip[cd + 42..]) as usize;
    ensure!(zip.len() >= local + 30 && &zip[local..local + 4] == b"PK\x03\x04", "zip: bad local header");
    let name_len = u16::from_le_bytes([zip[local + 26], zip[local + 27]]) as usize;
    let extra_len = u16::from_le_bytes([zip[local + 28], zip[local + 29]]) as usize;
    let start = local + 30 + name_len + extra_len;
    let body = zip.get(start..start + csize).context("zip: truncated entry")?;
    let mut out = Vec::with_capacity(usize_);
    match method {
        0 => out.extend_from_slice(body),
        8 => {
            flate2::read::DeflateDecoder::new(body).read_to_end(&mut out).context("zip: inflate")?;
        }
        m => bail!("zip: unsupported method {m}"),
    }
    ensure!(out.len() == usize_, "zip: size mismatch");
    Ok(out)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::io::Write;

    /// Build a one-entry zip (deflate) the way a zip tool would.
    pub fn make_zip(name: &str, data: &[u8], deflate: bool) -> Vec<u8> {
        let body = if deflate {
            let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(data).unwrap();
            e.finish().unwrap()
        } else {
            data.to_vec()
        };
        let method: u16 = if deflate { 8 } else { 0 };
        let crc = crc32fast::hash(data);
        let mut z = Vec::new();
        let local_at = 0u32;
        z.extend_from_slice(b"PK\x03\x04");
        z.extend_from_slice(&[20, 0, 0, 0]);
        z.extend_from_slice(&method.to_le_bytes());
        z.extend_from_slice(&[0; 4]);
        z.extend_from_slice(&crc.to_le_bytes());
        z.extend_from_slice(&(body.len() as u32).to_le_bytes());
        z.extend_from_slice(&(data.len() as u32).to_le_bytes());
        z.extend_from_slice(&(name.len() as u16).to_le_bytes());
        z.extend_from_slice(&[0, 0]);
        z.extend_from_slice(name.as_bytes());
        z.extend_from_slice(&body);
        let cd_at = z.len() as u32;
        z.extend_from_slice(b"PK\x01\x02");
        z.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
        z.extend_from_slice(&method.to_le_bytes());
        z.extend_from_slice(&[0; 4]);
        z.extend_from_slice(&crc.to_le_bytes());
        z.extend_from_slice(&(body.len() as u32).to_le_bytes());
        z.extend_from_slice(&(data.len() as u32).to_le_bytes());
        z.extend_from_slice(&(name.len() as u16).to_le_bytes());
        z.extend_from_slice(&[0; 12]);
        z.extend_from_slice(&local_at.to_le_bytes());
        z.extend_from_slice(name.as_bytes());
        let cd_len = z.len() as u32 - cd_at;
        z.extend_from_slice(b"PK\x05\x06\0\0\0\0\x01\0\x01\0");
        z.extend_from_slice(&cd_len.to_le_bytes());
        z.extend_from_slice(&cd_at.to_le_bytes());
        z.extend_from_slice(&[0, 0]);
        z
    }

    fn sample() -> Vec<u8> {
        (0..5000u32).map(|i| (i * i % 97) as u8).collect()
    }

    #[test]
    fn zip_both_methods() {
        for deflate in [true, false] {
            let z = make_zip("z", &sample(), deflate);
            assert_eq!(decompress(&z, 5000).unwrap(), sample());
        }
    }

    #[test]
    fn vzip_round_trip() {
        let data = sample();
        let mut lz = Vec::new();
        lzma_rs::lzma_compress_with_options(
            &mut &data[..],
            &mut lz,
            &lzma_rs::compress::Options { unpacked_size: lzma_rs::compress::UnpackedSize::SkipWritingToHeader },
        )
        .unwrap();
        // lzma-rs writes props(5) [+ size]; with the size skipped that is exactly the VZip layout.
        let mut v = b"VZa\0\0\0\0".to_vec();
        v.extend_from_slice(&lz);
        v.extend_from_slice(&crc32fast::hash(&data).to_le_bytes());
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(b"zv");
        assert_eq!(decompress(&v, data.len()).unwrap(), data);
        let n = v.len();
        v[n - 8] ^= 1; // corrupt CRC
        assert!(decompress(&v, data.len()).is_err());
    }

    #[test]
    fn vsz_round_trip() {
        let data = sample();
        let frame = ruzstd::encoding::compress_to_vec(&data[..], ruzstd::encoding::CompressionLevel::Fastest);
        let mut v = b"VSZa".to_vec();
        v.extend_from_slice(&crc32fast::hash(&data).to_le_bytes());
        v.extend_from_slice(&frame);
        v.extend_from_slice(&crc32fast::hash(&data).to_le_bytes());
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(&[0; 5]);
        v.extend_from_slice(b"zsv");
        assert_eq!(decompress(&v, data.len()).unwrap(), data);
        v[5] ^= 1; // wrong header CRC
        assert!(decompress(&v, data.len()).is_err());
    }

    #[test]
    fn rejects_unknown() {
        assert!(decompress(b"hello world", 11).is_err());
    }
}
