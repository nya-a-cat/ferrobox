use std::fmt;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const TOKEN_BYTES: usize = 32;

#[derive(Clone)]
pub(crate) struct TokenDigest([u8; 32]);

impl TokenDigest {
    pub(crate) fn issue() -> (String, Self) {
        let secret = rand::random::<[u8; TOKEN_BYTES]>();
        let plaintext = URL_SAFE_NO_PAD.encode(secret);
        let digest = Self::from_plaintext(&plaintext);
        (plaintext, digest)
    }

    pub(crate) fn matches(&self, candidate: &str) -> bool {
        let candidate = Self::from_plaintext(candidate);
        bool::from(self.0.ct_eq(&candidate.0))
    }

    fn from_plaintext(plaintext: &str) -> Self {
        Self(Sha256::digest(plaintext.as_bytes()).into())
    }
}

impl fmt::Debug for TokenDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TokenDigest([redacted])")
    }
}

#[cfg(test)]
mod tests {
    use super::TokenDigest;

    #[test]
    fn token_is_unique_and_verifiable() {
        let (first, digest) = TokenDigest::issue();
        let (second, _) = TokenDigest::issue();

        assert_ne!(first, second);
        assert!(digest.matches(&first));
        assert!(!digest.matches(&second));
        assert!(!format!("{digest:?}").contains(&first));
    }
}

