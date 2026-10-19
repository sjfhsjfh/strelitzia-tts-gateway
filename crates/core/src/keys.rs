use rand::RngExt;
use sha2::{Digest, Sha256};

pub const KEY_PREFIX: &str = "sk-";
pub const KEY_BODY_LEN: usize = 48;
pub const KEY_DISPLAY_PREFIX_LEN: usize = 8;

const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedKey {
    pub plaintext: String,
    pub hash: String,
    pub prefix: String,
}

pub fn hash_key(key: &str) -> String {
    hex::encode(Sha256::digest(key.as_bytes()))
}

pub fn generate_key() -> GeneratedKey {
    let mut rng = rand::rng();
    let body: String = (0..KEY_BODY_LEN)
        .map(|_| {
            let index = rng.random_range(0..ALPHABET.len());
            char::from(ALPHABET[index])
        })
        .collect();

    let plaintext = format!("{KEY_PREFIX}{body}");
    let prefix = body[..KEY_DISPLAY_PREFIX_LEN].to_owned();
    let hash = hash_key(&plaintext);

    GeneratedKey {
        plaintext,
        hash,
        prefix,
    }
}
