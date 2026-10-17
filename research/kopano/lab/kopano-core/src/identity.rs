//! Identity: accounts, devices and signatures.
//!
//! Three separate things (the mission's rule):
//! - **Account**: a person, as the identity provider knows them (in production: a ThabangVision
//!   account id). Kopano never holds the account password.
//! - **Device**: one computer, with its own ed25519 key. The same 32-byte key is the device's iroh
//!   endpoint id, so "who is connecting" and "who signed this version" are one fact, not two.
//! - **Device certificate**: the identity provider's signed statement "device key D belongs to
//!   account A until time T". Peers check it offline with the provider's public key, the way a
//!   browser checks a TLS certificate. Any provider can issue them (ThabangVision is one), which
//!   keeps Kopano open to third-party identity providers.
//!
//! Signatures are plain ed25519 (ed25519-dalek, audited) over canonical JSON. No custom crypto.

use anyhow::{bail, Context, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A value plus who signed it. The signature covers the canonical JSON of `body`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Signed<T> {
    pub body: T,
    /// Signer's ed25519 public key, hex.
    pub signer: String,
    pub sig: String,
}

impl<T: Serialize + DeserializeOwned> Signed<T> {
    pub fn new(body: T, key: &SigningKey) -> Result<Self> {
        let bytes = serde_json::to_vec(&body)?;
        Ok(Self {
            body,
            signer: hex::encode(key.verifying_key().to_bytes()),
            sig: hex::encode(key.sign(&bytes).to_bytes()),
        })
    }

    /// Check the signature. Returns the signer's key on success.
    pub fn verify(&self) -> Result<VerifyingKey> {
        let vk = parse_key(&self.signer)?;
        let sig: [u8; 64] = hex::decode(&self.sig)
            .ok()
            .and_then(|b| b.try_into().ok())
            .context("malformed signature")?;
        let bytes = serde_json::to_vec(&self.body)?;
        if vk.verify(&bytes, &Signature::from_bytes(&sig)).is_err() {
            bail!("signature check failed: the data was changed after it was signed");
        }
        Ok(vk)
    }
}

pub fn parse_key(hex_key: &str) -> Result<VerifyingKey> {
    let b: [u8; 32] = hex::decode(hex_key)
        .ok()
        .and_then(|b| b.try_into().ok())
        .context("malformed public key")?;
    VerifyingKey::from_bytes(&b).context("invalid public key")
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DeviceClaim {
    /// Format version of this certificate.
    pub v: u8,
    /// Stable account id at the identity provider.
    pub account: String,
    /// Display name, for history ("Thabang saved this").
    pub name: String,
    /// The device's public key (hex). Also its iroh endpoint id.
    pub device: String,
    pub device_label: String,
    pub issued_ms: u64,
    pub expires_ms: u64,
}

pub type DeviceCert = Signed<DeviceClaim>;

impl DeviceCert {
    /// Valid if signed by a trusted provider key and not expired at `at_ms`.
    pub fn check(&self, trusted_providers: &[String], at_ms: u64) -> Result<()> {
        self.verify()?;
        if !trusted_providers.iter().any(|k| k == &self.signer) {
            bail!("device certificate for {} is not signed by a trusted identity provider", self.body.name);
        }
        if at_ms > self.body.expires_ms {
            bail!("device certificate for {} ({}) has expired", self.body.name, self.body.device_label);
        }
        Ok(())
    }
}

/// One device's key. In the lab it is stored as a file; in production it belongs in the OS key
/// store (Windows Credential Manager / DPAPI, macOS Keychain, Secret Service).
pub struct DeviceKey {
    key: SigningKey,
}

impl DeviceKey {
    pub fn generate() -> Self {
        Self { key: SigningKey::generate(&mut rand::rngs::OsRng) }
    }
    pub fn from_bytes(b: [u8; 32]) -> Self {
        Self { key: SigningKey::from_bytes(&b) }
    }
    pub fn to_bytes(&self) -> [u8; 32] {
        self.key.to_bytes()
    }
    pub fn public_hex(&self) -> String {
        hex::encode(self.key.verifying_key().to_bytes())
    }
    pub fn signing(&self) -> &SigningKey {
        &self.key
    }
}

/// Stand-in for the identity provider's signing service (in production: ThabangVision's control
/// plane, issuing a certificate after the user signs in and registers the device).
pub struct Provider {
    key: SigningKey,
}

impl Provider {
    pub fn generate() -> Self {
        Self { key: SigningKey::generate(&mut rand::rngs::OsRng) }
    }
    pub fn from_bytes(b: [u8; 32]) -> Self {
        Self { key: SigningKey::from_bytes(&b) }
    }
    pub fn to_bytes(&self) -> [u8; 32] {
        self.key.to_bytes()
    }
    pub fn public_hex(&self) -> String {
        hex::encode(self.key.verifying_key().to_bytes())
    }
    pub fn issue(&self, account: &str, name: &str, device: &DeviceKey, label: &str, ttl_ms: u64) -> Result<DeviceCert> {
        let now = now_ms();
        Signed::new(
            DeviceClaim {
                v: 1,
                account: account.into(),
                name: name.into(),
                device: device.public_hex(),
                device_label: label.into(),
                issued_ms: now,
                expires_ms: now + ttl_ms,
            },
            &self.key,
        )
    }
}
