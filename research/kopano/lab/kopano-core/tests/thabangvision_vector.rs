//! ThabangVision issues Kopano device certificates from TypeScript (thabangvisionaistudios
//! lib/kopano/cert.ts). This test proves those certificates verify here, byte for byte, with the
//! same `DeviceCert::check` peers use. The vector file is a copy of the one in the ThabangVision
//! repo (tests/fixtures/kopano/device-cert-vector.json); both must stay identical.
//!
//! It also proves the other direction: signing the same claim with the same seed here gives the
//! same signature (ed25519 is deterministic), and the device-key signatures for the link token
//! and renewal messages match what kopanod will produce with `SigningKey::sign`.

use ed25519_dalek::{Signer, SigningKey};
use kopano_core::identity::{DeviceCert, DeviceClaim, Signed};
use serde_json::Value;

fn vector() -> Value {
    let text = include_str!("vectors/thabangvision-device-cert.json");
    serde_json::from_str(text).expect("vector is JSON")
}

fn seed(v: &Value, field: &str) -> SigningKey {
    let b: [u8; 32] = hex::decode(v[field].as_str().unwrap()).unwrap().try_into().unwrap();
    SigningKey::from_bytes(&b)
}

#[test]
fn thabangvision_certificate_verifies_with_kopano_core() {
    let v = vector();
    let cert: DeviceCert = serde_json::from_value(v["cert"].clone()).expect("cert parses as DeviceCert");
    let issuer = v["issuer_public"].as_str().unwrap().to_string();

    // The bytes serde_json signs are exactly the ones the TypeScript side signed.
    let ours = String::from_utf8(serde_json::to_vec(&cert.body).unwrap()).unwrap();
    assert_eq!(ours, v["signed_json"].as_str().unwrap());

    // Peer check: good now, refused after expiry, refused when the issuer is not trusted.
    cert.check(&[issuer.clone()], v["at_ms_valid"].as_u64().unwrap()).expect("valid certificate");
    assert!(cert.check(&[issuer.clone()], v["at_ms_expired"].as_u64().unwrap()).is_err());
    assert!(cert.check(&["00".repeat(32)], v["at_ms_valid"].as_u64().unwrap()).is_err());

    // Any change to the body breaks the signature.
    let mut tampered = cert.clone();
    tampered.body.account.push('x');
    assert!(tampered.verify().is_err());
}

#[test]
fn same_seed_same_signature_both_sides() {
    let v = vector();
    let body: DeviceClaim = serde_json::from_value(v["cert"]["body"].clone()).unwrap();
    let issuer = seed(&v, "issuer_seed");
    let signed = Signed::new(body, &issuer).unwrap();
    assert_eq!(signed.signer, v["issuer_public"].as_str().unwrap());
    assert_eq!(signed.sig, v["cert"]["sig"].as_str().unwrap());
}

#[test]
fn device_signatures_match_what_kopanod_will_send() {
    let v = vector();
    let device = seed(&v, "device_seed");
    assert_eq!(hex::encode(device.verifying_key().to_bytes()), v["device_public"].as_str().unwrap());

    let token_msg = format!("kopano-link-token:v1:{}", v["token"]["device_code"].as_str().unwrap());
    assert_eq!(token_msg, v["token"]["message"].as_str().unwrap());
    assert_eq!(hex::encode(device.sign(token_msg.as_bytes()).to_bytes()), v["token"]["signature"].as_str().unwrap());

    let renew_msg = format!(
        "kopano-cert-renew:v1:{}:{}",
        v["device_public"].as_str().unwrap(),
        v["renew"]["timestamp"].as_u64().unwrap()
    );
    assert_eq!(renew_msg, v["renew"]["message"].as_str().unwrap());
    assert_eq!(hex::encode(device.sign(renew_msg.as_bytes()).to_bytes()), v["renew"]["signature"].as_str().unwrap());
}
