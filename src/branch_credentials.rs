use crate::state::StateStore;
use anyhow::{Context, Result};
use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce,
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
};

const FORMAT_VERSION: u8 = 1;
const KEYRING_USER: &str = "gog-branch-password-key";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialErrorKind {
    MissingKey,
    CorruptCredential,
    UnsupportedFormat,
}

#[derive(Debug)]
pub struct CredentialError(pub CredentialErrorKind);

impl std::fmt::Display for CredentialError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self.0 {
            CredentialErrorKind::MissingKey => {
                "protected branch credential key is unavailable; enter the password again"
            }
            CredentialErrorKind::CorruptCredential => {
                "protected branch credential cannot be decrypted; enter the password again"
            }
            CredentialErrorKind::UnsupportedFormat => {
                "protected branch credential format is unsupported; enter the password again"
            }
        })
    }
}

impl std::error::Error for CredentialError {}

pub fn save(
    store: &StateStore,
    user_id: &str,
    product_id: i64,
    branch: &str,
    password: &str,
) -> Result<()> {
    let key = load_or_create_key()?;
    let (nonce, ciphertext) = encrypt(&key, user_id, product_id, branch, password)?;
    store.save_galaxy_branch_credential(
        user_id,
        product_id,
        branch,
        FORMAT_VERSION,
        &nonce,
        &ciphertext,
    )
}

pub fn load(
    store: &StateStore,
    user_id: &str,
    product_id: i64,
    branch: &str,
) -> Result<Option<String>> {
    let Some((version, nonce, ciphertext)) =
        store.galaxy_branch_credential(user_id, product_id, branch)?
    else {
        return Ok(None);
    };
    if version != FORMAT_VERSION {
        return Err(CredentialError(CredentialErrorKind::UnsupportedFormat).into());
    }
    let Some(key) = load_key()? else {
        return Err(CredentialError(CredentialErrorKind::MissingKey).into());
    };
    decrypt(&key, user_id, product_id, branch, &nonce, &ciphertext)
        .map(Some)
        .map_err(|_| CredentialError(CredentialErrorKind::CorruptCredential).into())
}

pub fn forget(store: &StateStore, user_id: &str, product_id: i64, branch: &str) -> Result<()> {
    store.delete_galaxy_branch_credential(user_id, product_id, branch)
}

pub fn forget_all(store: &StateStore, user_id: &str) -> Result<usize> {
    store.delete_all_galaxy_branch_credentials(user_id)
}

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(crate::identity::APP_ID, KEYRING_USER).map_err(Into::into)
}

fn load_key() -> Result<Option<[u8; 32]>> {
    match entry()?.get_password() {
        Ok(serialized) => {
            let bytes: Vec<u8> = serde_json::from_str(&serialized)
                .context("decoding protected branch credential key")?;
            Ok(Some(bytes.try_into().map_err(|_| {
                anyhow::anyhow!("protected branch credential key has an invalid length")
            })?))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn load_or_create_key() -> Result<[u8; 32]> {
    if let Some(key) = load_key()? {
        return Ok(key);
    }
    let key = ChaCha20Poly1305::generate_key(&mut OsRng);
    entry()?.set_password(&serde_json::to_string(key.as_slice())?)?;
    Ok(key.into())
}

fn associated_data(user_id: &str, product_id: i64, branch: &str) -> Result<Vec<u8>> {
    serde_json::to_vec(&(FORMAT_VERSION, user_id, product_id, branch)).map_err(Into::into)
}

fn encrypt(
    key: &[u8; 32],
    user_id: &str,
    product_id: i64,
    branch: &str,
    password: &str,
) -> Result<([u8; 12], Vec<u8>)> {
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = ChaCha20Poly1305::new(Key::from_slice(key))
        .encrypt(
            &nonce,
            Payload {
                msg: password.as_bytes(),
                aad: &associated_data(user_id, product_id, branch)?,
            },
        )
        .map_err(|_| anyhow::anyhow!("could not encrypt protected branch credential"))?;
    Ok((nonce.into(), ciphertext))
}

fn decrypt(
    key: &[u8; 32],
    user_id: &str,
    product_id: i64,
    branch: &str,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<String> {
    let nonce: &[u8; 12] = nonce
        .try_into()
        .map_err(|_| anyhow::anyhow!("protected branch credential nonce has an invalid length"))?;
    let plaintext = ChaCha20Poly1305::new(Key::from_slice(key))
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: &associated_data(user_id, product_id, branch)?,
            },
        )
        .map_err(|_| anyhow::anyhow!("could not decrypt protected branch credential"))?;
    String::from_utf8(plaintext).context("protected branch credential is not valid UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ciphertext_is_random_and_bound_to_its_identity() {
        let key = [7; 32];
        let first = encrypt(&key, "user", 42, "beta", "secret").unwrap();
        let second = encrypt(&key, "user", 42, "beta", "secret").unwrap();
        assert_ne!(first, second);
        assert_eq!(
            decrypt(&key, "user", 42, "beta", &first.0, &first.1).unwrap(),
            "secret"
        );
        assert!(decrypt(&key, "other", 42, "beta", &first.0, &first.1).is_err());
        assert!(decrypt(&key, "user", 43, "beta", &first.0, &first.1).is_err());
        assert!(decrypt(&key, "user", 42, "other", &first.0, &first.1).is_err());
    }

    #[test]
    fn tampering_is_rejected() {
        let key = [9; 32];
        let (nonce, mut ciphertext) = encrypt(&key, "user", 42, "beta", "secret").unwrap();
        ciphertext[0] ^= 1;
        assert!(decrypt(&key, "user", 42, "beta", &nonce, &ciphertext).is_err());
    }
}
