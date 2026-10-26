use super::*;

#[test]
fn blowfish_round_trips() {
    let key = b"83657ea6ffa1e671375c689a2e99a598";
    let plain = b"LOA Font Studio!";
    let encrypted = blowfish_ecb(plain, key, true).unwrap();
    assert_ne!(encrypted, plain);
    let decrypted = blowfish_ecb(&encrypted, key, false).unwrap();
    assert_eq!(decrypted, plain);
}

#[test]
fn aes_restarts_the_iv_every_kib() {
    use aes::Aes256;
    use aes::cipher::{Array, BlockCipherEncrypt, KeyInit};

    let key = [7u8; 32];
    let plain = (0..3072u32)
        .map(|value| (value % 251) as u8)
        .collect::<Vec<_>>();

    let cipher = Aes256::new_from_slice(&key).unwrap();
    let mut encrypted = plain.clone();
    for block in encrypted.as_chunks_mut::<1024>().0 {
        let mut previous = [0u8; 16];
        for chunk in block.as_chunks_mut::<16>().0 {
            let mut bytes: [u8; 16] = *chunk;
            for (index, byte) in bytes.iter_mut().enumerate() {
                *byte ^= previous[index];
            }
            let mut block = Array::from(bytes);
            cipher.encrypt_block(&mut block);
            chunk.copy_from_slice(block.as_slice());
            previous.copy_from_slice(chunk);
        }
    }

    assert_eq!(aes_cbc_decrypt(&encrypted, &key).unwrap(), plain);
}

#[test]
fn derives_the_known_entry_key() {
    let key = derive_entry_key(
        "\\Binaries\\Fonts\\English\\ExuberanceF-Regular.ttf",
        "1069d88738c5c75f82b44a1f0a382768",
    )
    .unwrap();
    assert_eq!(
        hex_lower(&key),
        "c217a0aea0733eb134608b6deed2e7d45cb6d7bb7e747fc9ebe1ef5ebc527de6"
    );
}
