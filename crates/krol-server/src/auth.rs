//! Login authentication: the RSA key exchange, the session server check and offline UUIDs
//! (vanilla: `CryptManager`, `NetHandlerLoginServer`, authlib's `YggdrasilMinecraftSessionService`).

use md5::{Digest as _, Md5};
use rsa::pkcs8::EncodePublicKey;
use rsa::{BoxedUint, PrivateKey};
use sha1::Sha1;
use thiserror::Error;

/// The server's RSA key pair, generated at startup ("Generating keypair").
pub struct ServerKeyPair {
    private: PrivateKey,
    public_der: Vec<u8>,
}

impl std::fmt::Debug for ServerKeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerKeyPair")
            .field("public_der_len", &self.public_der.len())
            .finish_non_exhaustive()
    }
}

/// Errors from cryptographic operations.
#[derive(Debug, Error)]
pub enum CryptoError {
    /// Key generation or encoding failed.
    #[error("key generation failed: {0}")]
    KeyGeneration(String),
    /// RSA decryption failed (vanilla logs "Cipher data failed!").
    #[error("Cipher data failed!")]
    Decrypt,
}

impl ServerKeyPair {
    /// Generates a 1024-bit RSA key pair with public exponent 65537, as vanilla does.
    ///
    /// # Errors
    ///
    /// [`CryptoError::KeyGeneration`] if the OS RNG or encoding fails.
    pub fn generate() -> Result<Self, CryptoError> {
        let private =
            PrivateKey::new_with_exp(&mut getrandom::SysRng, 1024, BoxedUint::from(65_537u32))
                .map_err(|e| CryptoError::KeyGeneration(e.to_string()))?;
        let public_der = private
            .to_public_key()
            .to_public_key_der()
            .map_err(|e| CryptoError::KeyGeneration(e.to_string()))?
            .as_bytes()
            .to_vec();
        Ok(Self {
            private,
            public_der,
        })
    }

    /// The X.509 `SubjectPublicKeyInfo` DER encoding sent in the Encryption Request.
    pub fn public_der(&self) -> &[u8] {
        &self.public_der
    }

    /// Decrypts PKCS#1 v1.5 data (`RSA/ECB/PKCS1Padding`).
    ///
    /// On bad padding this may return pseudo-random bytes instead of an error (implicit
    /// rejection, a timing side-channel defence); callers must validate the result, as
    /// the verify token comparison does.
    ///
    /// # Errors
    ///
    /// [`CryptoError::Decrypt`] on invalid input length.
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        self.private
            .pkcs1v15_decrypt(data)
            .map_err(|_| CryptoError::Decrypt)
    }

    /// Encrypts with the public key (used by test clients).
    ///
    /// # Errors
    ///
    /// [`CryptoError::Decrypt`] if the data is too long for the key.
    pub fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        self.private
            .to_public_key()
            .pkcs1v15_encrypt(data)
            .map_err(|_| CryptoError::Decrypt)
    }
}

/// The server id hash sent to the session server: SHA-1 over `server_id` (ISO-8859-1),
/// the shared secret and the public key, printed as Java's `new BigInteger(hash)
/// .toString(16)` (signed, no leading zeros).
pub fn server_id_hash(server_id: &str, shared_secret: &[u8], public_der: &[u8]) -> String {
    let mut sha = Sha1::new();
    let latin1: Vec<u8> = server_id.chars().map(|c| c as u32 as u8).collect();
    sha.update(&latin1);
    sha.update(shared_secret);
    sha.update(public_der);
    let digest: [u8; 20] = sha.finalize().into();
    java_signed_hex(&digest)
}

/// Formats big-endian two's-complement bytes like `new BigInteger(bytes).toString(16)`.
pub fn java_signed_hex(bytes: &[u8]) -> String {
    let negative = bytes.first().is_some_and(|b| b & 0x80 != 0);
    let mut magnitude = bytes.to_vec();
    if negative {
        // Two's complement negation.
        let mut carry = true;
        for b in magnitude.iter_mut().rev() {
            *b = !*b;
            if carry {
                let (v, c) = b.overflowing_add(1);
                *b = v;
                carry = c;
            }
        }
    }
    let hex: String = magnitude.iter().map(|b| format!("{b:02x}")).collect();
    let trimmed = hex.trim_start_matches('0');
    let digits = if trimmed.is_empty() { "0" } else { trimmed };
    if negative {
        format!("-{digits}")
    } else {
        digits.to_owned()
    }
}

/// The UUID vanilla assigns to `name` in offline mode:
/// `UUID.nameUUIDFromBytes(("OfflinePlayer:" + name).getBytes(UTF_8))` (version 3).
pub fn offline_uuid(name: &str) -> u128 {
    let mut hash: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
    hash[6] = (hash[6] & 0x0f) | 0x30;
    hash[8] = (hash[8] & 0x3f) | 0x80;
    u128::from_be_bytes(hash)
}

/// A profile property (e.g. `textures`) returned by the session server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileProperty {
    /// Property name.
    pub name: String,
    /// Base64 value.
    pub value: String,
    /// Yggdrasil signature, if present.
    pub signature: Option<String>,
}

/// An authenticated (or offline) game profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameProfile {
    /// Player UUID.
    pub id: u128,
    /// Player name.
    pub name: String,
    /// Profile properties (empty in offline mode).
    pub properties: Vec<ProfileProperty>,
}

/// Outcome of a session server check.
#[derive(Debug)]
pub enum SessionResult {
    /// The player joined with a valid session.
    Joined(GameProfile),
    /// No valid session (vanilla: "Failed to verify username!").
    NotJoined,
    /// The session server could not be reached (vanilla: "Authentication servers are
    /// down. Please try again later, sorry!").
    Unavailable(String),
}

/// Base URL of Mojang's session server.
pub const SESSION_SERVER: &str = "https://sessionserver.mojang.com/session/minecraft/hasJoined";

/// Asks the session server whether `name` joined with `server_hash` (blocking HTTP).
pub fn has_joined(base_url: &str, name: &str, server_hash: &str) -> SessionResult {
    let url = format!(
        "{base_url}?username={}&serverId={}",
        url_encode(name),
        url_encode(server_hash)
    );
    let response = match ureq::get(&url).call() {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(_)) => return SessionResult::NotJoined,
        Err(e) => return SessionResult::Unavailable(e.to_string()),
    };
    if response.status().as_u16() == 204 {
        return SessionResult::NotJoined;
    }
    let body = match response.into_body().read_to_string() {
        Ok(b) => b,
        Err(e) => return SessionResult::Unavailable(e.to_string()),
    };
    match parse_profile(&body) {
        Some(profile) => SessionResult::Joined(profile),
        None => SessionResult::NotJoined,
    }
}

fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'*' => {
                char::from(b).to_string()
            }
            b' ' => "+".to_owned(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Parses a session server profile (`{"id": "<32 hex>", "name": ..., "properties": [...]}`).
pub fn parse_profile(json: &str) -> Option<GameProfile> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let id = u128::from_str_radix(v.get("id")?.as_str()?, 16).ok()?;
    let name = v.get("name")?.as_str()?.to_owned();
    let properties = v
        .get("properties")
        .and_then(|p| p.as_array())
        .map(|props| {
            props
                .iter()
                .filter_map(|p| {
                    Some(ProfileProperty {
                        name: p.get("name")?.as_str()?.to_owned(),
                        value: p.get("value")?.as_str()?.to_owned(),
                        signature: p
                            .get("signature")
                            .and_then(|s| s.as_str())
                            .map(str::to_owned),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(GameProfile {
        id,
        name,
        properties,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Well-known examples of Minecraft's hex digest (SHA-1 of the name alone).
    #[test]
    fn minecraft_hex_digest_examples() {
        let digest = |s: &str| {
            let d: [u8; 20] = Sha1::digest(s.as_bytes()).into();
            java_signed_hex(&d)
        };
        assert_eq!(digest("Notch"), "4ed1f46bbe04bc756bcb17c0c7ce3e4632f06a48");
        assert_eq!(digest("jeb_"), "-7c9d5b0044c130109a5d7b5fb5c317c02b4e28c1");
        assert_eq!(digest("simon"), "88e16a1019277b15d58faf0541e11910eb756f6");
    }

    #[test]
    fn offline_uuids_match_java() {
        // UUID.nameUUIDFromBytes("OfflinePlayer:Notch".getBytes(UTF_8))
        assert_eq!(
            crate::status::java_uuid_string(offline_uuid("Notch")),
            "b50ad385-829d-3141-a216-7e7d7539ba7f"
        );
    }

    #[test]
    fn key_exchange_round_trip() {
        let keys = ServerKeyPair::generate().unwrap();
        assert!(keys.public_der().len() > 150);
        let secret = [5u8; 16];
        let encrypted = keys.encrypt(&secret).unwrap();
        assert_eq!(encrypted.len(), 128);
        assert_eq!(keys.decrypt(&encrypted).unwrap(), secret);
        // Bad padding never yields the real plaintext (rsa uses implicit rejection, so it
        // may return pseudo-random bytes instead of an error; the verify token check then
        // fails exactly as vanilla's does).
        assert_ne!(keys.decrypt(&[1u8; 128]).ok(), Some(secret.to_vec()));
    }

    #[test]
    fn parses_profiles() {
        let p = parse_profile(
            r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"Notch","properties":[{"name":"textures","value":"abc","signature":"sig"}]}"#,
        )
        .unwrap();
        assert_eq!(p.id, 0x069a79f4_44e9_4726_a5be_fca90e38aaf5);
        assert_eq!(p.properties[0].signature.as_deref(), Some("sig"));
        assert!(parse_profile("{}").is_none());
    }
}
