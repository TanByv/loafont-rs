use aes::Aes256;
use aes::cipher::{Array, BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use anyhow::{Result, bail};
use blowfish::BlowfishLE;
use md5::{Digest, Md5};
use sha2::Sha256;

/// QuickBMS uses OpenSSL's Blowfish implementation, which reads and writes
/// 32-bit words in little-endian order. `BlowfishLE` matches that behaviour.
pub fn blowfish_ecb(data: &[u8], key: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    if !data.len().is_multiple_of(8) {
        bail!("Blowfish data is not aligned to 8 bytes.");
    }

    let cipher = BlowfishLE::new_from_slice(key)
        .map_err(|_| anyhow::anyhow!("Invalid Blowfish key length."))?;
    let mut output = data.to_vec();

    for chunk in output.as_chunks_mut::<8>().0 {
        let mut block = Array::from(*chunk);
        if encrypt {
            cipher.encrypt_block(&mut block);
        } else {
            cipher.decrypt_block(&mut block);
        }
        chunk.copy_from_slice(block.as_slice());
    }

    Ok(output)
}

/// AES-256-CBC with a zero IV. The game applies a fresh IV to every 1024-byte
/// block, but CBC state is continuous inside each block.
pub fn aes_cbc_decrypt(data: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    if !data.len().is_multiple_of(16) {
        bail!("AES data is not aligned to 16 bytes.");
    }

    let cipher =
        Aes256::new_from_slice(key).map_err(|_| anyhow::anyhow!("Invalid AES key length."))?;
    let mut output = data.to_vec();
    let mut previous = [0u8; 16];

    for (index, chunk) in output.as_chunks_mut::<16>().0.iter_mut().enumerate() {
        if index % 64 == 0 {
            previous = [0u8; 16];
        }
        let ciphertext: [u8; 16] = *chunk;
        let mut block = Array::from(ciphertext);
        cipher.decrypt_block(&mut block);
        for (i, byte) in chunk.iter_mut().enumerate() {
            *byte = block[i] ^ previous[i];
        }
        previous = ciphertext;
    }

    Ok(output)
}

/// Reproduces the per-entry AES key derivation from the original BMS script.
/// The archive entry name is reduced to the part after the last underscore and
/// before the first dot, converted to UTF-16LE, hashed with MD5, then XORed
/// against the client's BASE value. The XOR result is hex encoded and hashed
/// with SHA-256 to produce the AES key.
pub fn derive_entry_key(name: &str, base_hex: &str) -> Result<[u8; 32]> {
    let base = hex_decode(base_hex)?;
    if base.len() < 16 {
        bail!("Client BASE key is too short.");
    }

    let file = name.rsplit(['\\', '/']).next().unwrap_or(name);
    let after_underscore = file.rsplit_once('_').map_or(file, |(_, rest)| rest);
    let stem = after_underscore
        .split_once('.')
        .map_or(after_underscore, |(stem, _)| stem);

    let utf16: Vec<u8> = stem
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    let digest = Md5::digest(&utf16);

    let mut material = [0u8; 16];
    for (i, byte) in material.iter_mut().enumerate() {
        *byte = base[i] ^ digest[15 - i];
    }

    let hash = Sha256::digest(hex_lower(&material).as_bytes());
    let mut key = [0u8; 32];
    key.copy_from_slice(&hash);
    Ok(key)
}

fn hex_decode(value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        bail!("Hex value has an odd length.");
    }

    (0..value.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&value[i..i + 2], 16)
                .map_err(|_| anyhow::anyhow!("Invalid hex value: {value}"))
        })
        .collect()
}

fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(text, "{byte:02x}").expect("writing to a String cannot fail");
    }
    text
}

#[cfg(test)]
#[path = "../tests/unit/crypto.rs"]
mod tests;
