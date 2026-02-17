use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};

/// PKCE (Proof Key for Code Exchange) challenge pair.
#[derive(Debug, Clone)]
pub struct PkceChallenge {
    pub verifier: String,
    pub challenge: String,
}

/// Generate a PKCE verifier and challenge.
pub fn generate_pkce() -> PkceChallenge {
    use rand::Rng;
    let mut rng = rand::rng();
    let verifier_bytes: Vec<u8> = (0..32).map(|_| rng.random::<u8>()).collect();
    let verifier = URL_SAFE_NO_PAD.encode(&verifier_bytes);

    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let digest = hasher.finalize();
    let challenge = URL_SAFE_NO_PAD.encode(digest);

    PkceChallenge {
        verifier,
        challenge,
    }
}
