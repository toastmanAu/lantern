//! Secret-bearing types. All retained secret material in the vault lives
//! inside `SecretBox<T>` so accidental `Debug`/`Display`/`serde` exposure is
//! impossible at the type level.
//!
//! `MasterKey` wraps a 32-byte Argon2id-derived key. It zeroizes on drop
//! (via the `ZeroizeOnDrop` derive, which `SecretBox` already applies, but
//! we also put it on the inner type as belt+braces for any direct
//! construction during tests).
//!
//! `InnerStore` is the serialisable shape of the decrypted vault payload: a
//! map of named blobs. It is wrapped in `SecretBox<InnerStore>` inside
//! `Vault` so no code path can accidentally log it.

use secrecy::SecretBox;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct MasterKey(pub [u8; 32]);

impl MasterKey {
    pub fn into_secret(self) -> SecretBox<[u8; 32]> {
        SecretBox::new(Box::new(self.0))
    }
}

/// Inner vault payload. A map of blob-name → raw bytes. The concrete type
/// stored under each key is the caller's concern (they serialise whatever
/// they want before calling `Vault::put`).
///
/// **Cloning is expensive AND duplicates secret material.** Every blob value
/// is copied. Prefer borrowing via `Vault::get` → `&[u8]`-style APIs when
/// possible.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct InnerStore {
    pub blobs: BTreeMap<String, Vec<u8>>,
}

impl InnerStore {
    pub fn new() -> Self {
        Self::default()
    }
}

// `zeroize`'s derive cannot see through `BTreeMap<K, V>` (the crate only
// supports Vec, arrays, primitives, Option, String, tuples). We implement
// `Zeroize` by hand to preserve the spec's memory-hygiene contract: every
// secret-bearing `Vec<u8>` value is wiped, then the map is cleared. Keys
// (blob names like "mnemonic" or "ckb-key-0") are not secret material so
// we don't need to zeroize them individually — dropping the `String`
// deallocates its heap storage.
impl Zeroize for InnerStore {
    fn zeroize(&mut self) {
        for v in self.blobs.values_mut() {
            v.zeroize();
        }
        self.blobs.clear();
    }
}

impl Drop for InnerStore {
    fn drop(&mut self) {
        self.zeroize();
    }
}

// Marker: upholds the `Drop` + `zeroize()` contract above.
impl ZeroizeOnDrop for InnerStore {}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[test]
    fn master_key_round_trips_through_secret_box() {
        let k = MasterKey([0xAB; 32]);
        let boxed = k.into_secret();
        assert_eq!(boxed.expose_secret(), &[0xAB; 32]);
    }

    #[test]
    fn inner_store_roundtrips_cbor() {
        let mut s = InnerStore::new();
        s.blobs
            .insert("mnemonic".into(), b"abandon abandon ...".to_vec());
        let mut buf: Vec<u8> = Vec::new();
        ciborium::into_writer(&s, &mut buf).unwrap();
        let decoded: InnerStore = ciborium::from_reader(buf.as_slice()).unwrap();
        assert_eq!(decoded.blobs, s.blobs);
    }

    #[test]
    fn inner_store_zeroize_clears_blobs() {
        let mut s = InnerStore::new();
        s.blobs.insert("a".into(), vec![0x41, 0x42, 0x43]);
        s.blobs.insert("b".into(), vec![0x44, 0x45]);
        s.zeroize();
        assert!(s.blobs.is_empty(), "zeroize() must clear the map");
    }

    #[test]
    fn secret_box_has_opaque_debug() {
        let boxed = MasterKey([0xAB; 32]).into_secret();
        let s = format!("{boxed:?}");
        assert!(s.contains("SecretBox"), "debug must be opaque, got: {s}");
        assert!(!s.contains("AB"), "debug leaked key bytes: {s}");
    }
}
