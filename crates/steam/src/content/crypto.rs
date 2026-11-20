//! Depot encryption. Steam encrypts file names (base64) and every chunk with
//! the depot key using one scheme: the first 16 bytes are an IV that is itself
//! AES-256-ECB encrypted with the key; the rest is AES-256-CBC with PKCS7.

use aes::Aes256;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecrypt, BlockDecryptMut, KeyInit, KeyIvInit, block_padding::Pkcs7};
use anyhow::{Result, anyhow, bail};
use base64::Engine as _;

pub type DepotKey = [u8; 32];

/// Decrypt a blob in Steam's "IV, then CBC" layout.
pub fn decrypt(key: &DepotKey, data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 32 || !data.len().is_multiple_of(16) {
        bail!("encrypted blob has bad length {}", data.len());
    }
    let (iv_enc, body) = data.split_at(16);
    let mut iv = GenericArray::clone_from_slice(iv_enc);
    Aes256::new(GenericArray::from_slice(key)).decrypt_block(&mut iv);
    cbc::Decryptor::<Aes256>::new(GenericArray::from_slice(key), &iv)
        .decrypt_padded_vec_mut::<Pkcs7>(body)
        .map_err(|_| anyhow!("bad padding (wrong depot key?)"))
}

/// `decrypt_filename_raw` with `/` separators.
#[cfg(test)]
pub fn decrypt_filename(key: &DepotKey, b64: &str) -> Result<String> {
    Ok(decrypt_filename_raw(key, b64)?.replace('\\', "/"))
}

/// Decrypt a manifest file name: base64, then `decrypt`, then cut at the NUL.
/// Keeps the backslashes Windows-made depots use.
pub fn decrypt_filename_raw(key: &DepotKey, b64: &str) -> Result<String> {
    let raw = base64::engine::general_purpose::STANDARD.decode(b64.trim())?;
    let mut plain = decrypt(key, &raw)?;
    if let Some(nul) = plain.iter().position(|&b| b == 0) {
        plain.truncate(nul);
    }
    Ok(String::from_utf8_lossy(&plain).into_owned())
}

#[cfg(test)]
pub fn encrypt(key: &DepotKey, iv: [u8; 16], data: &[u8]) -> Vec<u8> {
    use aes::cipher::{BlockEncrypt, BlockEncryptMut};
    let mut iv_block = GenericArray::clone_from_slice(&iv);
    Aes256::new(GenericArray::from_slice(key)).encrypt_block(&mut iv_block);
    let body = cbc::Encryptor::<Aes256>::new(GenericArray::from_slice(key), GenericArray::from_slice(&iv))
        .encrypt_padded_vec_mut::<Pkcs7>(data);
    [iv_block.as_slice(), &body].concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: DepotKey = [7; 32];

    #[test]
    fn blob_round_trip() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let enc = encrypt(&KEY, [3; 16], &data);
        assert_eq!(decrypt(&KEY, &enc).unwrap(), data);
        assert!(decrypt(&[8; 32], &enc).is_err() || decrypt(&[8; 32], &enc).unwrap() != data);
    }

    #[test]
    fn filename_round_trip() {
        let enc = encrypt(&KEY, [9; 16], b"bin\\linux64\\tool.so\0");
        let b64 = base64::engine::general_purpose::STANDARD.encode(enc);
        assert_eq!(decrypt_filename(&KEY, &b64).unwrap(), "bin/linux64/tool.so");
    }
}
