//! Sealing a file so the relay stores bytes it cannot read.
//!
//! Summo's whole claim is that your recordings are yours. Sync is the one feature that sends a
//! vault somewhere else, so it is the one place that claim can quietly become false — and the way
//! it becomes false is not malice, it is a support engineer with database access, a backup that
//! outlives the company, or a subpoena. Encrypting on this side removes the question rather than
//! answering it.
//!
//! ```text
//!   passphrase ──Argon2id──► key ──┬─► XChaCha20-Poly1305 ──► ciphertext
//!                                  └─► BLAKE3(key, path)   ──► opaque id
//! ```
//!
//! ## What is hidden, and what is not
//!
//! **Contents** are encrypted, and so are **names**: a relay that could read
//! `meetings/2026-08-10-acquisition-with-vinacapital.md` would learn most of what matters without
//! decrypting a byte. The path is hashed with a key derived from the passphrase, so the same file
//! maps to the same id on every machine that has the passphrase and to nothing at all without it.
//!
//! **Sizes and timing are not hidden.** The relay has to store and address blobs, so it sees how
//! many there are, how big, and when they changed. Padding to hide sizes would multiply storage for
//! a threat model this product does not have, and pretending otherwise would be worse than saying
//! so plainly.
//!
//! ## The key never leaves
//!
//! There is no recovery. A passphrase that a server could reset is a passphrase the server knows,
//! which is the arrangement this module exists to avoid. Losing it means losing the ability to read
//! what was uploaded — the local vault is untouched, because the local vault is the real one.

use argon2::Argon2;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use summo_core::{Error, Result};

/// Bytes of the nonce XChaCha20-Poly1305 uses.
///
/// Extended, 24 bytes, specifically so a random nonce is safe. The 12-byte variant needs a counter
/// nobody loses track of, and "nobody loses track of a counter across three machines and a restore
/// from backup" is not a design, it is a hope.
const NONCE_LEN: usize = 24;

/// Argon2id cost. Deliberately slow: this runs once per unlock, and the passphrase is the only
/// thing between an attacker with the ciphertext and the contents.
const MEMORY_KIB: u32 = 64 * 1024;
const ITERATIONS: u32 = 3;
const PARALLELISM: u32 = 4;

/// A key derived from the user's passphrase. Never serialised, never sent.
#[derive(Clone)]
pub struct Key {
    bytes: [u8; 32],
}

/// Printed as a fact, not a value. One stray `{:?}` in a log line should not be a key disclosure.
impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(<redacted>)")
    }
}

impl Key {
    /// Derive from a passphrase and a salt.
    ///
    /// The salt is per-vault, stored in the clear beside the sync state. It is not a secret; it
    /// exists so two people who choose the same passphrase do not produce the same key, and so a
    /// precomputed table cannot cover both.
    pub fn derive(passphrase: &str, salt: &[u8]) -> Result<Self> {
        if passphrase.trim().is_empty() {
            return Err(Error::Config("a sync passphrase cannot be empty".into()));
        }
        if salt.len() < 8 {
            return Err(Error::Config("the sync salt is too short".into()));
        }

        let params = argon2::Params::new(MEMORY_KIB, ITERATIONS, PARALLELISM, Some(32))
            .map_err(|e| Error::Other(format!("cannot configure the key derivation: {e}")))?;
        let argon = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);

        let mut bytes = [0u8; 32];
        argon
            .hash_password_into(passphrase.as_bytes(), salt, &mut bytes)
            .map_err(|e| Error::Other(format!("cannot derive the sync key: {e}")))?;
        Ok(Self { bytes })
    }

    /// The id a path is stored under on the relay.
    ///
    /// Keyed, so it cannot be reversed by hashing a guess: without the passphrase,
    /// `meetings/2026-08-10-sync.md` and any other path are equally unrecognisable. Deterministic,
    /// so two machines with the same passphrase agree on where a file lives without coordinating.
    #[must_use]
    pub fn id_for(&self, path: &str) -> String {
        blake3::keyed_hash(&self.bytes, path.as_bytes())
            .to_hex()
            .to_string()
    }

    /// Encrypt, binding the ciphertext to the path it belongs to.
    ///
    /// The path travels as associated data rather than in the ciphertext: it is already known to
    /// whoever holds the key, and binding it means a relay cannot serve one file's contents under
    /// another's id without the decryption failing.
    pub fn seal(&self, path: &str, plaintext: &[u8]) -> Result<Sealed> {
        let cipher = XChaCha20Poly1305::new_from_slice(&self.bytes)
            .map_err(|e| Error::Other(format!("cannot build the cipher: {e}")))?;

        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce)
            .map_err(|e| Error::Other(format!("cannot generate a nonce: {e}")))?;

        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: path.as_bytes(),
                },
            )
            .map_err(|_| Error::Other("cannot encrypt".into()))?;

        Ok(Sealed { nonce, ciphertext })
    }

    /// Decrypt, checking it is the file it claims to be.
    ///
    /// A failure here is never "the wrong bytes came back". Poly1305 makes tampering and a wrong
    /// key indistinguishable from each other and from noise, which is the point: the caller has no
    /// decision to make except to stop.
    pub fn open(&self, path: &str, sealed: &Sealed) -> Result<Vec<u8>> {
        let cipher = XChaCha20Poly1305::new_from_slice(&self.bytes)
            .map_err(|e| Error::Other(format!("cannot build the cipher: {e}")))?;

        cipher
            .decrypt(
                XNonce::from_slice(&sealed.nonce),
                Payload {
                    msg: &sealed.ciphertext,
                    aad: path.as_bytes(),
                },
            )
            .map_err(|_| {
                Error::msg(
                    "sync.cannot_open",
                    "không giải mã được — sai passphrase, hoặc dữ liệu đã bị sửa".to_string(),
                )
            })
    }
}

/// One encrypted file, as it travels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>,
}

impl Sealed {
    /// Nonce first, then ciphertext. The nonce is not secret and has to arrive with the message.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(NONCE_LEN + self.ciphertext.len());
        out.extend_from_slice(&self.nonce);
        out.extend_from_slice(&self.ciphertext);
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() <= NONCE_LEN {
            return Err(Error::msg(
                "sync.truncated",
                "khối dữ liệu quá ngắn để hợp lệ".to_string(),
            ));
        }
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&bytes[..NONCE_LEN]);
        Ok(Self {
            nonce,
            ciphertext: bytes[NONCE_LEN..].to_vec(),
        })
    }
}

/// A fresh per-vault salt.
pub fn new_salt() -> Result<[u8; 16]> {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|e| Error::Other(format!("cannot generate a salt: {e}")))?;
    Ok(salt)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cheap parameters. The real ones take ~100 ms per derivation by design, which is right for a
    /// user typing a passphrase once and wrong for a test suite deriving keys in a loop.
    fn key(passphrase: &str) -> Key {
        let params = argon2::Params::new(8, 1, 1, Some(32)).unwrap();
        let argon = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
        let mut bytes = [0u8; 32];
        argon
            .hash_password_into(passphrase.as_bytes(), b"a-fixed-test-salt", &mut bytes)
            .unwrap();
        Key { bytes }
    }

    #[test]
    fn a_file_survives_a_round_trip() {
        let key = key("đúng ngựa pin");
        let sealed = key.seal("meetings/a.md", b"# Hop\n\nnoi dung").unwrap();
        let opened = key.open("meetings/a.md", &sealed).unwrap();
        assert_eq!(opened, b"# Hop\n\nnoi dung");
    }

    #[test]
    fn an_empty_file_survives_a_round_trip() {
        let key = key("p");
        let sealed = key.seal("a.md", b"").unwrap();
        assert_eq!(key.open("a.md", &sealed).unwrap(), b"");
    }

    /// The relay stores this. If it were readable, everything else here is theatre.
    #[test]
    fn the_ciphertext_does_not_contain_the_plaintext() {
        let key = key("p");
        let secret = b"acquisition with VinaCapital at 40 million";
        let sealed = key.seal("meetings/a.md", secret).unwrap();

        let haystack = sealed.to_bytes();
        assert!(
            !haystack.windows(secret.len()).any(|w| w == secret),
            "the plaintext is in the blob"
        );
    }

    #[test]
    fn the_wrong_passphrase_fails_rather_than_returning_rubbish() {
        let sealed = key("right").seal("a.md", b"secret").unwrap();
        let err = key("wrong").open("a.md", &sealed).unwrap_err();
        assert_eq!(err.code(), Some("sync.cannot_open"), "{err}");
    }

    /// A relay serving one file's contents under another file's id must fail, not succeed quietly.
    #[test]
    fn contents_are_bound_to_the_path_they_belong_to() {
        let key = key("p");
        let sealed = key.seal("meetings/salaries.md", b"payroll").unwrap();
        assert!(key.open("meetings/agenda.md", &sealed).is_err());
    }

    #[test]
    fn a_tampered_blob_is_rejected() {
        let key = key("p");
        let mut sealed = key.seal("a.md", b"hello").unwrap();
        let last = sealed.ciphertext.len() - 1;
        sealed.ciphertext[last] ^= 0x01;
        assert!(key.open("a.md", &sealed).is_err());
    }

    /// A repeated nonce with the same key leaks the XOR of two plaintexts. Random 24-byte nonces
    /// are why this is safe without a counter that has to survive a restore from backup.
    #[test]
    fn sealing_the_same_file_twice_produces_different_bytes() {
        let key = key("p");
        let once = key.seal("a.md", b"same contents").unwrap();
        let twice = key.seal("a.md", b"same contents").unwrap();
        assert_ne!(once.nonce, twice.nonce);
        assert_ne!(once.ciphertext, twice.ciphertext);
    }

    // ---- names ---------------------------------------------------------------------------

    /// A relay that could read `2026-08-10-acquisition-with-vinacapital.md` would learn most of
    /// what matters without decrypting a byte.
    #[test]
    fn a_stored_id_reveals_nothing_about_the_path() {
        let id = key("p").id_for("meetings/2026-08-10-acquisition.md");
        assert!(!id.contains("acquisition"));
        assert!(!id.contains("meetings"));
        assert_eq!(id.len(), 64);
    }

    /// Two machines with the same passphrase have to agree on where a file lives without talking.
    #[test]
    fn the_same_path_and_passphrase_always_give_the_same_id() {
        assert_eq!(key("p").id_for("a.md"), key("p").id_for("a.md"));
    }

    #[test]
    fn different_paths_give_different_ids() {
        let key = key("p");
        assert_ne!(key.id_for("a.md"), key.id_for("b.md"));
    }

    /// Keyed, so an id cannot be reversed by hashing a guessed path.
    #[test]
    fn a_different_passphrase_gives_a_different_id_for_the_same_path() {
        assert_ne!(key("one").id_for("a.md"), key("two").id_for("a.md"));
    }

    // ---- the wire format -------------------------------------------------------------------

    #[test]
    fn a_sealed_blob_survives_being_written_and_read_back() {
        let key = key("p");
        let sealed = key.seal("a.md", b"contents").unwrap();
        let back = Sealed::from_bytes(&sealed.to_bytes()).unwrap();
        assert_eq!(back, sealed);
        assert_eq!(key.open("a.md", &back).unwrap(), b"contents");
    }

    #[test]
    fn a_truncated_blob_is_refused_rather_than_panicking() {
        assert!(Sealed::from_bytes(&[]).is_err());
        assert!(Sealed::from_bytes(&[0u8; NONCE_LEN]).is_err());
        assert!(Sealed::from_bytes(&[0u8; NONCE_LEN + 1]).is_ok());
    }

    // ---- derivation ------------------------------------------------------------------------

    #[test]
    fn a_blank_passphrase_is_refused() {
        assert!(Key::derive("", b"0123456789abcdef").is_err());
        assert!(Key::derive("   ", b"0123456789abcdef").is_err());
    }

    #[test]
    fn a_short_salt_is_refused() {
        assert!(Key::derive("p", b"short").is_err());
    }

    /// Two people who choose the same passphrase must not produce the same key.
    #[test]
    fn the_salt_changes_the_key() {
        let one = Key::derive("same passphrase", b"0123456789abcdef").unwrap();
        let two = Key::derive("same passphrase", b"fedcba9876543210").unwrap();
        assert_ne!(one.id_for("a.md"), two.id_for("a.md"));
    }

    #[test]
    fn a_fresh_salt_is_not_the_same_twice() {
        assert_ne!(new_salt().unwrap(), new_salt().unwrap());
    }

    /// One stray `{:?}` in a log line should not be a key disclosure.
    #[test]
    fn a_key_does_not_print_itself() {
        let text = format!("{:?}", key("p"));
        assert_eq!(text, "Key(<redacted>)");
    }
}
