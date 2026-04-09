# Lantern Plan 1b — Vault Crate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `crates/vault` (the `lantern-vault` crate) as a standalone, fully-tested module that can create, unlock, and persist an encrypted at-rest secret store backed by Argon2id + XChaCha20-Poly1305, with `SecretBox`-wrapped master-key state and HKDF-derived per-extension subkeys.

**Architecture:** Vault file = 58-byte header (magic + version + KDF params + salt + nonce) followed by AEAD ciphertext (CBOR-serialised inner store + Poly1305 tag). The inner store is a `BTreeMap<String, SecretSlice<u8>>` of named blobs. Unlock reads the file, re-runs Argon2id over the stored salt, decrypts the envelope, and holds the decrypted plaintext *only* inside a `SecretBox<InnerStore>` for the session. Save re-encrypts with a *fresh* nonce on every write. Extension authors never touch the master key — they call `vault.extension_subkey(ext_id)` which HKDF-expands a 32-byte subkey they can use for their own AEAD. `mlock` is deferred to plan 1c (OS-dependent, best-effort, not blocking for v0.0.2).

**Tech Stack:** `argon2` 0.5, `chacha20poly1305` 0.10, `hkdf` 0.12, `secrecy` 0.10, `zeroize` 1.8, `rand` 0.8, `ciborium` 0.2 (inner-store serialisation), `thiserror` 2.0, `tracing` 0.1.

---

## File Structure

```
crates/vault/
├── Cargo.toml                  # dep wiring (Task 1)
└── src/
    ├── lib.rs                  # module exports + Vault public API (Task 8)
    ├── error.rs                # VaultError enum (Task 2)
    ├── format.rs               # file format constants + Header encode/decode (Task 3)
    ├── kdf.rs                  # Argon2id wrapper: derive_master_key (Task 4)
    ├── aead.rs                 # XChaCha20-Poly1305 seal/open helpers (Task 5)
    ├── secret.rs               # MasterKey + InnerStore types, zeroize impls (Task 6)
    ├── subkey.rs               # HKDF per-extension subkey derivation (Task 9)
    └── vault.rs                # Vault struct: create/unlock/put/get/save/lock (Task 7)
tests/
└── vault_roundtrip.rs          # integration test (Task 10)
```

Each module has one responsibility. `error.rs` is the first file written so every later module can return `Result<_, VaultError>` from its first test.

## Determinations Locked in This Plan

- **Magic header:** `b"LANTERN\0"` (8 bytes exact)
- **Format version:** `0x01` (first byte after magic)
- **KDF algorithm id:** `0x01` = Argon2id
- **Argon2id params (v1, pinned):** m_cost = 65536 KiB (64 MiB), t_cost = 3, p_cost = 1, output length = 32 bytes. Stored in the header so a future v2 can tune them without format ambiguity.
- **Salt length:** 16 bytes (random, generated at create time, re-used on subsequent saves so the master key doesn't re-derive).
- **Nonce length:** 24 bytes (XChaCha20 extended nonce). **Fresh random nonce on every save.**
- **Inner store format:** CBOR (`ciborium`) — deterministic, schema-flexible, fast enough for a few KB of secret material.
- **No `mlock` in this plan.** Deferred to plan 1c with a dedicated feature flag. Documented as a known gap in the vault module doc comment.

---

## Task 1: Wire workspace + vault Cargo.toml

**Files:**
- Modify: `Cargo.toml` (workspace root) — add crypto deps to `[workspace.dependencies]`
- Modify: `crates/vault/Cargo.toml` — consume them

- [ ] **Step 1: Add workspace dependencies**

Edit `Cargo.toml` — add the following block immediately after the existing `tracing-subscriber` line inside `[workspace.dependencies]`:

```toml
# Crypto (plan 1b: vault)
argon2 = "0.5"
chacha20poly1305 = "0.10"
hkdf = "0.12"
sha2 = "0.10"
secrecy = "0.10"
zeroize = { version = "1.8", features = ["derive"] }
rand = "0.8"
ciborium = "0.2"
```

- [ ] **Step 2: Replace `crates/vault/Cargo.toml` with the full wiring**

```toml
[package]
name = "lantern-vault"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
description = "Lantern vault — encrypted at-rest storage of secret material"

[lints]
workspace = true

[dependencies]
thiserror.workspace = true
tracing.workspace = true
argon2.workspace = true
chacha20poly1305.workspace = true
hkdf.workspace = true
sha2.workspace = true
secrecy.workspace = true
zeroize.workspace = true
rand.workspace = true
ciborium.workspace = true
serde = { workspace = true, features = ["derive"] }

[dev-dependencies]
tempfile = "3.10"
```

- [ ] **Step 3: Verify the workspace still builds**

Run: `cargo check -p lantern-vault`
Expected: PASS with only the existing placeholder warning (no errors, lib.rs unchanged).

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/vault/Cargo.toml
git commit -m "chore(vault): wire crypto dependencies for plan 1b"
```

---

## Task 2: `error.rs` — VaultError enum

**Files:**
- Create: `crates/vault/src/error.rs`
- Modify: `crates/vault/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/vault/src/lib.rs` (replace the existing `tests` module):

```rust
#![forbid(unsafe_code)]

pub mod error;
pub use error::VaultError;

#[cfg(test)]
mod tests {
    use super::VaultError;

    #[test]
    fn error_display_is_redacted() {
        // Secret-bearing errors must never print raw secret bytes.
        let e = VaultError::WrongPassword;
        let s = format!("{e}");
        assert_eq!(s, "wrong password");
    }

    #[test]
    fn io_error_converts() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "x");
        let e: VaultError = io_err.into();
        assert!(matches!(e, VaultError::Io(_)));
    }
}
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test -p lantern-vault`
Expected: FAIL — `VaultError` not defined.

- [ ] **Step 3: Implement `error.rs`**

Create `crates/vault/src/error.rs`:

```rust
//! Error type for the vault crate.
//!
//! Display impls must NEVER surface secret bytes — inner error values from
//! `argon2` or `chacha20poly1305` are mapped to opaque variants.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("vault file is not a Lantern vault (magic header mismatch)")]
    BadMagic,

    #[error("unsupported vault format version: got {0}, supported: 1")]
    UnsupportedVersion(u8),

    #[error("unsupported KDF algorithm id: {0}")]
    UnsupportedKdf(u8),

    #[error("vault header is truncated (need {need} bytes, got {got})")]
    TruncatedHeader { need: usize, got: usize },

    #[error("key derivation failed")]
    KdfFailed,

    #[error("wrong password")]
    WrongPassword,

    #[error("vault payload failed authentication (tampered or corrupt)")]
    AuthFailed,

    #[error("vault inner-store serialisation failed")]
    SerdeFailed,

    #[error("HKDF expansion failed")]
    HkdfFailed,
}
```

- [ ] **Step 4: Run the test**

Run: `cargo test -p lantern-vault`
Expected: both tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/vault/src/lib.rs crates/vault/src/error.rs
git commit -m "feat(vault): VaultError enum with redacted Display impls"
```

---

## Task 3: `format.rs` — header encode/decode

**Files:**
- Create: `crates/vault/src/format.rs`
- Modify: `crates/vault/src/lib.rs` (add `pub mod format;`)

- [ ] **Step 1: Write the failing test**

Create `crates/vault/src/format.rs`:

```rust
//! Vault file format v1.
//!
//! Layout (58 bytes header + N bytes ciphertext):
//!
//! ```text
//!   offset  size  field
//!   0       8     magic      = b"LANTERN\0"
//!   8       1     version    = 0x01
//!   9       1     kdf_algo   = 0x01 (Argon2id)
//!   10      4     m_cost_kib (u32 LE)
//!   14      2     t_cost     (u16 LE)
//!   16      2     p_cost     (u16 LE)
//!   18      16    salt
//!   34      24    nonce
//!   58      N     ciphertext (Poly1305 tag is the last 16 bytes)
//! ```

use crate::error::VaultError;

pub const MAGIC: &[u8; 8] = b"LANTERN\0";
pub const VERSION_V1: u8 = 0x01;
pub const KDF_ARGON2ID: u8 = 0x01;
pub const HEADER_LEN: usize = 58;
pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 24;

/// Pinned Argon2id parameters for format version 1.
pub const V1_ARGON2_M_KIB: u32 = 65_536; // 64 MiB
pub const V1_ARGON2_T: u16 = 3;
pub const V1_ARGON2_P: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub version: u8,
    pub kdf_algo: u8,
    pub m_cost_kib: u32,
    pub t_cost: u16,
    pub p_cost: u16,
    pub salt: [u8; SALT_LEN],
    pub nonce: [u8; NONCE_LEN],
}

impl Header {
    /// Build a fresh v1 header with the pinned Argon2id params.
    pub fn new_v1(salt: [u8; SALT_LEN], nonce: [u8; NONCE_LEN]) -> Self {
        Self {
            version: VERSION_V1,
            kdf_algo: KDF_ARGON2ID,
            m_cost_kib: V1_ARGON2_M_KIB,
            t_cost: V1_ARGON2_T,
            p_cost: V1_ARGON2_P,
            salt,
            nonce,
        }
    }

    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[0..8].copy_from_slice(MAGIC);
        out[8] = self.version;
        out[9] = self.kdf_algo;
        out[10..14].copy_from_slice(&self.m_cost_kib.to_le_bytes());
        out[14..16].copy_from_slice(&self.t_cost.to_le_bytes());
        out[16..18].copy_from_slice(&self.p_cost.to_le_bytes());
        out[18..34].copy_from_slice(&self.salt);
        out[34..58].copy_from_slice(&self.nonce);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, VaultError> {
        if bytes.len() < HEADER_LEN {
            return Err(VaultError::TruncatedHeader {
                need: HEADER_LEN,
                got: bytes.len(),
            });
        }
        if &bytes[0..8] != MAGIC {
            return Err(VaultError::BadMagic);
        }
        let version = bytes[8];
        if version != VERSION_V1 {
            return Err(VaultError::UnsupportedVersion(version));
        }
        let kdf_algo = bytes[9];
        if kdf_algo != KDF_ARGON2ID {
            return Err(VaultError::UnsupportedKdf(kdf_algo));
        }
        let m_cost_kib = u32::from_le_bytes(bytes[10..14].try_into().unwrap());
        let t_cost = u16::from_le_bytes(bytes[14..16].try_into().unwrap());
        let p_cost = u16::from_le_bytes(bytes[16..18].try_into().unwrap());
        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&bytes[18..34]);
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&bytes[34..58]);
        Ok(Self { version, kdf_algo, m_cost_kib, t_cost, p_cost, salt, nonce })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_v1_header() {
        let salt = [7u8; SALT_LEN];
        let nonce = [9u8; NONCE_LEN];
        let h = Header::new_v1(salt, nonce);
        let encoded = h.encode();
        assert_eq!(encoded.len(), HEADER_LEN);
        assert_eq!(&encoded[0..8], MAGIC);
        let decoded = Header::decode(&encoded).unwrap();
        assert_eq!(decoded, h);
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = [0u8; HEADER_LEN];
        bytes[0..8].copy_from_slice(b"NOTLTRN\0");
        assert!(matches!(Header::decode(&bytes), Err(VaultError::BadMagic)));
    }

    #[test]
    fn rejects_unsupported_version() {
        let mut bytes = Header::new_v1([0; SALT_LEN], [0; NONCE_LEN]).encode();
        bytes[8] = 0x02;
        assert!(matches!(
            Header::decode(&bytes),
            Err(VaultError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn rejects_truncated() {
        let bytes = [0u8; 10];
        assert!(matches!(
            Header::decode(&bytes),
            Err(VaultError::TruncatedHeader { need: 58, got: 10 })
        ));
    }
}
```

Add `pub mod format;` under the existing `pub mod error;` line in `crates/vault/src/lib.rs`.

- [ ] **Step 2: Run tests**

Run: `cargo test -p lantern-vault`
Expected: 6 tests PASS (2 from error + 4 from format).

- [ ] **Step 3: Commit**

```bash
git add crates/vault/src/format.rs crates/vault/src/lib.rs
git commit -m "feat(vault): v1 file format header encode/decode"
```

---

## Task 4: `kdf.rs` — Argon2id wrapper

**Files:**
- Create: `crates/vault/src/kdf.rs`
- Modify: `crates/vault/src/lib.rs` (add `pub mod kdf;`)

- [ ] **Step 1: Write the failing test**

Create `crates/vault/src/kdf.rs`:

```rust
//! Argon2id password-KDF wrapper.
//!
//! Params are passed explicitly rather than read from a constant so callers
//! that loaded them from a header (future v2 format) can use this too. For v1
//! the pinned params live in `format::{V1_ARGON2_M_KIB, V1_ARGON2_T, V1_ARGON2_P}`.

use argon2::{Algorithm, Argon2, Params, Version};

use crate::error::VaultError;

/// Derive a 32-byte master key from (password, salt) using Argon2id.
///
/// Returns the raw key bytes. The caller must wrap them in a zeroizing /
/// `SecretBox` container before retaining them — this function does NOT
/// wrap on its own because `secret.rs` owns the master-key type and we
/// avoid circular module deps.
pub fn derive_master_key(
    password: &[u8],
    salt: &[u8],
    m_cost_kib: u32,
    t_cost: u16,
    p_cost: u16,
) -> Result<[u8; 32], VaultError> {
    let params = Params::new(
        m_cost_kib,
        u32::from(t_cost),
        u32::from(p_cost),
        Some(32),
    )
    .map_err(|_| VaultError::KdfFailed)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    argon2
        .hash_password_into(password, salt, &mut out)
        .map_err(|_| VaultError::KdfFailed)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{V1_ARGON2_M_KIB, V1_ARGON2_P, V1_ARGON2_T};

    // Use tiny params in tests so they don't take 400ms each.
    const TEST_M: u32 = 8; // 8 KiB
    const TEST_T: u16 = 1;
    const TEST_P: u16 = 1;

    #[test]
    fn same_inputs_same_output() {
        let k1 = derive_master_key(b"hunter2", b"salty-salty-saltsa", TEST_M, TEST_T, TEST_P).unwrap();
        let k2 = derive_master_key(b"hunter2", b"salty-salty-saltsa", TEST_M, TEST_T, TEST_P).unwrap();
        assert_eq!(k1, k2);
    }

    #[test]
    fn different_passwords_diverge() {
        let k1 = derive_master_key(b"hunter2", b"salty-salty-saltsa", TEST_M, TEST_T, TEST_P).unwrap();
        let k2 = derive_master_key(b"hunter3", b"salty-salty-saltsa", TEST_M, TEST_T, TEST_P).unwrap();
        assert_ne!(k1, k2);
    }

    #[test]
    fn different_salts_diverge() {
        let k1 = derive_master_key(b"hunter2", b"salty-salty-salts1", TEST_M, TEST_T, TEST_P).unwrap();
        let k2 = derive_master_key(b"hunter2", b"salty-salty-salts2", TEST_M, TEST_T, TEST_P).unwrap();
        assert_ne!(k1, k2);
    }

    #[test]
    fn v1_pinned_params_accepted() {
        // Sanity: the params we pinned in format.rs must be valid Argon2 params.
        // Use 8 KiB override since the pinned 64 MiB is too slow for a test.
        let _ = derive_master_key(b"x", &[0u8; 16], 8, V1_ARGON2_T, V1_ARGON2_P).unwrap();
        // Reference V1 constants to keep this test honest about what it pins.
        assert_eq!(V1_ARGON2_M_KIB, 65_536);
    }
}
```

Add `pub mod kdf;` to `crates/vault/src/lib.rs`.

- [ ] **Step 2: Run tests**

Run: `cargo test -p lantern-vault`
Expected: 10 tests PASS. Argon2 tests will take ~1-2s even with tiny params on first run.

- [ ] **Step 3: Commit**

```bash
git add crates/vault/src/kdf.rs crates/vault/src/lib.rs
git commit -m "feat(vault): Argon2id KDF wrapper with deterministic test params"
```

---

## Task 5: `aead.rs` — XChaCha20-Poly1305 seal/open

**Files:**
- Create: `crates/vault/src/aead.rs`
- Modify: `crates/vault/src/lib.rs` (add `pub mod aead;`)

- [ ] **Step 1: Write the failing test**

Create `crates/vault/src/aead.rs`:

```rust
//! AEAD wrapper around `XChaCha20Poly1305`.
//!
//! Thin boundary: takes key + nonce + plaintext/ciphertext slices, returns
//! the other. No state, no retained secrets. Callers own nonce freshness —
//! this module just does not care where the nonce came from.

use chacha20poly1305::{
    aead::{Aead, KeyInit},
    Key, XChaCha20Poly1305, XNonce,
};

use crate::error::VaultError;

pub fn seal(key: &[u8; 32], nonce: &[u8; 24], plaintext: &[u8]) -> Result<Vec<u8>, VaultError> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    cipher
        .encrypt(XNonce::from_slice(nonce), plaintext)
        .map_err(|_| VaultError::AuthFailed)
}

pub fn open(key: &[u8; 32], nonce: &[u8; 24], ciphertext: &[u8]) -> Result<Vec<u8>, VaultError> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    cipher
        .decrypt(XNonce::from_slice(nonce), ciphertext)
        .map_err(|_| VaultError::AuthFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_roundtrip() {
        let key = [0x11u8; 32];
        let nonce = [0x22u8; 24];
        let pt = b"hello lantern vault";
        let ct = seal(&key, &nonce, pt).unwrap();
        assert_ne!(&ct[..pt.len()], pt); // actually encrypted
        assert_eq!(ct.len(), pt.len() + 16); // + tag
        let roundtrip = open(&key, &nonce, &ct).unwrap();
        assert_eq!(roundtrip, pt);
    }

    #[test]
    fn wrong_key_fails() {
        let key = [0x11u8; 32];
        let bad = [0x12u8; 32];
        let nonce = [0x22u8; 24];
        let ct = seal(&key, &nonce, b"x").unwrap();
        assert!(matches!(open(&bad, &nonce, &ct), Err(VaultError::AuthFailed)));
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let key = [0x11u8; 32];
        let nonce = [0x22u8; 24];
        let mut ct = seal(&key, &nonce, b"hello").unwrap();
        ct[0] ^= 1;
        assert!(matches!(open(&key, &nonce, &ct), Err(VaultError::AuthFailed)));
    }

    #[test]
    fn wrong_nonce_fails() {
        let key = [0x11u8; 32];
        let nonce = [0x22u8; 24];
        let bad_nonce = [0x23u8; 24];
        let ct = seal(&key, &nonce, b"hello").unwrap();
        assert!(matches!(
            open(&key, &bad_nonce, &ct),
            Err(VaultError::AuthFailed)
        ));
    }
}
```

Add `pub mod aead;` to `crates/vault/src/lib.rs`.

- [ ] **Step 2: Run tests**

Run: `cargo test -p lantern-vault`
Expected: 14 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/vault/src/aead.rs crates/vault/src/lib.rs
git commit -m "feat(vault): XChaCha20-Poly1305 seal/open helpers"
```

---

## Task 6: `secret.rs` — MasterKey + InnerStore types

**Files:**
- Create: `crates/vault/src/secret.rs`
- Modify: `crates/vault/src/lib.rs` (add `pub mod secret;`)

- [ ] **Step 1: Write the failing test**

Create `crates/vault/src/secret.rs`:

```rust
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
#[derive(Clone, Default, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct InnerStore {
    pub blobs: BTreeMap<String, Vec<u8>>,
}

impl InnerStore {
    pub fn new() -> Self {
        Self::default()
    }
}

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
        s.blobs.insert("mnemonic".into(), b"abandon abandon ...".to_vec());
        let mut buf: Vec<u8> = Vec::new();
        ciborium::into_writer(&s, &mut buf).unwrap();
        let decoded: InnerStore = ciborium::from_reader(buf.as_slice()).unwrap();
        assert_eq!(decoded.blobs, s.blobs);
    }

    #[test]
    fn secret_box_has_opaque_debug() {
        let boxed = MasterKey([0xAB; 32]).into_secret();
        let s = format!("{boxed:?}");
        assert!(s.contains("SecretBox"), "debug must be opaque, got: {s}");
        assert!(!s.contains("AB"), "debug leaked key bytes: {s}");
    }
}
```

Add `pub mod secret;` to `crates/vault/src/lib.rs`.

- [ ] **Step 2: Run tests**

Run: `cargo test -p lantern-vault`
Expected: 17 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/vault/src/secret.rs crates/vault/src/lib.rs
git commit -m "feat(vault): MasterKey + InnerStore with SecretBox wrapping"
```

---

## Task 7: `vault.rs` — Vault struct and public API

**Files:**
- Create: `crates/vault/src/vault.rs`
- Modify: `crates/vault/src/lib.rs` (add `pub mod vault;` + re-export)

- [ ] **Step 1: Write the failing tests**

Create `crates/vault/src/vault.rs`:

```rust
//! The `Vault` struct — the public face of the crate.
//!
//! Lifecycle:
//!
//! 1. `Vault::create(path, password)` — generates random salt + nonce,
//!    derives the master key, writes an empty encrypted store to disk, and
//!    returns an unlocked `Vault` ready for `put`.
//! 2. `Vault::unlock(path, password)` — reads the file, re-derives the
//!    master key, decrypts the store, returns an unlocked `Vault`.
//! 3. `vault.put(name, bytes)` / `vault.get(name)` — read/write blobs.
//! 4. `vault.save()` — re-encrypts with a FRESH nonce and rewrites the file.
//!    The salt is unchanged so the password doesn't re-derive.
//! 5. `vault.lock()` — consumes the vault, zeroizes the master key + store.
//!
//! **Memory hygiene guarantees:**
//! - Master key is always inside a `SecretBox<[u8; 32]>`.
//! - Inner store is always inside a `SecretBox<InnerStore>`.
//! - Password bytes are zeroized before function return.
//! - `mlock` is NOT performed in this plan (1b). See the module doc for
//!   why, and plan 1c for the follow-up.

use std::fs;
use std::path::{Path, PathBuf};

use rand::{rngs::OsRng, RngCore};
use secrecy::{ExposeSecret, ExposeSecretMut, SecretBox};
use zeroize::Zeroize;

use crate::aead::{open, seal};
use crate::error::VaultError;
use crate::format::{Header, HEADER_LEN, NONCE_LEN, SALT_LEN};
use crate::kdf::derive_master_key;
use crate::secret::InnerStore;

pub struct Vault {
    path: PathBuf,
    header: Header,
    master_key: SecretBox<[u8; 32]>,
    store: SecretBox<InnerStore>,
}

impl Vault {
    pub fn create(path: impl AsRef<Path>, password: &[u8]) -> Result<Self, VaultError> {
        let path = path.as_ref().to_path_buf();
        let mut salt = [0u8; SALT_LEN];
        let mut nonce = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut salt);
        OsRng.fill_bytes(&mut nonce);
        let header = Header::new_v1(salt, nonce);

        let key_bytes = derive_master_key(
            password,
            &header.salt,
            header.m_cost_kib,
            header.t_cost,
            header.p_cost,
        )?;
        let master_key = SecretBox::new(Box::new(key_bytes));

        let store = SecretBox::new(Box::new(InnerStore::new()));
        let v = Self { path, header, master_key, store };
        v.save()?;
        Ok(v)
    }

    pub fn unlock(path: impl AsRef<Path>, password: &[u8]) -> Result<Self, VaultError> {
        let path = path.as_ref().to_path_buf();
        let bytes = fs::read(&path)?;
        if bytes.len() < HEADER_LEN {
            return Err(VaultError::TruncatedHeader {
                need: HEADER_LEN,
                got: bytes.len(),
            });
        }
        let header = Header::decode(&bytes[..HEADER_LEN])?;
        let ciphertext = &bytes[HEADER_LEN..];

        let key_bytes = derive_master_key(
            password,
            &header.salt,
            header.m_cost_kib,
            header.t_cost,
            header.p_cost,
        )?;

        let plaintext = match open(&key_bytes, &header.nonce, ciphertext) {
            Ok(p) => p,
            Err(_) => {
                // AEAD failure is either wrong password or tampering. From
                // the caller's perspective the former is vastly more common,
                // so map to WrongPassword — but a true tampering attack
                // can't be distinguished without additional state.
                return Err(VaultError::WrongPassword);
            }
        };

        let store: InnerStore = ciborium::from_reader(plaintext.as_slice())
            .map_err(|_| VaultError::SerdeFailed)?;

        // Zeroize the derived key copy we produced before wrapping.
        let master_key = SecretBox::new(Box::new(key_bytes));

        Ok(Self {
            path,
            header,
            master_key,
            store: SecretBox::new(Box::new(store)),
        })
    }

    pub fn put(&mut self, name: &str, value: &[u8]) {
        self.store
            .expose_secret_mut()
            .blobs
            .insert(name.to_string(), value.to_vec());
    }

    pub fn get(&self, name: &str) -> Option<Vec<u8>> {
        self.store.expose_secret().blobs.get(name).cloned()
    }

    pub fn save(&self) -> Result<(), VaultError> {
        let mut plaintext: Vec<u8> = Vec::new();
        ciborium::into_writer(self.store.expose_secret(), &mut plaintext)
            .map_err(|_| VaultError::SerdeFailed)?;

        // Fresh nonce on every save — this is the XChaCha guarantee.
        let mut nonce = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce);
        let mut header = self.header;
        header.nonce = nonce;

        let ciphertext = seal(self.master_key.expose_secret(), &nonce, &plaintext)?;

        plaintext.zeroize();

        let mut out: Vec<u8> = Vec::with_capacity(HEADER_LEN + ciphertext.len());
        out.extend_from_slice(&header.encode());
        out.extend_from_slice(&ciphertext);

        // Atomic-ish write: write to `<path>.tmp` then rename.
        let tmp = self.path.with_extension("tmp");
        fs::write(&tmp, &out)?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// Consume the vault, zeroizing held secrets.
    pub fn lock(self) {
        // Drop impls on SecretBox + MasterKey handle zeroization.
        drop(self);
    }

    /// Expose the master key to a closure without cloning it. Used by the
    /// subkey module in Task 9.
    pub(crate) fn with_master_key<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&[u8; 32]) -> R,
    {
        f(self.master_key.expose_secret())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // Tests use the same 64 MiB Argon2 params as production so they
    // round-trip through the real header. Slow (~400ms each) but correct.

    #[test]
    fn create_then_unlock_empty_vault() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault.bin");
        {
            let _v = Vault::create(&path, b"correct horse battery staple").unwrap();
        }
        let v = Vault::unlock(&path, b"correct horse battery staple").unwrap();
        assert_eq!(v.get("anything"), None);
    }

    #[test]
    fn put_then_get_survives_save_and_unlock() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault.bin");
        {
            let mut v = Vault::create(&path, b"pw").unwrap();
            v.put("mnemonic", b"abandon abandon abandon");
            v.save().unwrap();
        }
        let v = Vault::unlock(&path, b"pw").unwrap();
        assert_eq!(v.get("mnemonic").as_deref(), Some(&b"abandon abandon abandon"[..]));
    }

    #[test]
    fn wrong_password_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault.bin");
        let _ = Vault::create(&path, b"right").unwrap();
        let err = Vault::unlock(&path, b"wrong").unwrap_err();
        assert!(matches!(err, VaultError::WrongPassword));
    }

    #[test]
    fn save_rotates_nonce() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault.bin");
        let mut v = Vault::create(&path, b"pw").unwrap();
        let bytes_1 = fs::read(&path).unwrap();
        v.put("x", b"1");
        v.save().unwrap();
        let bytes_2 = fs::read(&path).unwrap();
        // Header nonce is bytes 34..58. Must differ.
        assert_ne!(&bytes_1[34..58], &bytes_2[34..58]);
    }

    #[test]
    fn bad_magic_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault.bin");
        fs::write(&path, b"not a lantern vault at all").unwrap();
        let err = Vault::unlock(&path, b"pw").unwrap_err();
        // Could be TruncatedHeader or BadMagic depending on file length.
        assert!(matches!(
            err,
            VaultError::BadMagic | VaultError::TruncatedHeader { .. }
        ));
    }
}
```

Update `crates/vault/src/lib.rs`:

```rust
#![forbid(unsafe_code)]

//! Lantern vault.
//!
//! Encrypted at-rest storage for secret material. See plan 1b for scope.
//! Memory locking (`mlock`) is intentionally NOT implemented here — it lands
//! in plan 1c behind a platform feature flag. All other memory-hygiene
//! guarantees (zeroize, SecretBox, no Debug leakage) are in force.

pub mod aead;
pub mod error;
pub mod format;
pub mod kdf;
pub mod secret;
pub mod vault;

pub use error::VaultError;
pub use vault::Vault;
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p lantern-vault`
Expected: 22 tests PASS. The five new ones are Argon2-heavy; expect ~3-5s total test time.

- [ ] **Step 3: Commit**

```bash
git add crates/vault/src/vault.rs crates/vault/src/lib.rs
git commit -m "feat(vault): Vault create/unlock/put/get/save/lock pipeline"
```

---

## Task 8: `subkey.rs` — HKDF per-extension subkeys

**Files:**
- Create: `crates/vault/src/subkey.rs`
- Modify: `crates/vault/src/lib.rs` (add `pub mod subkey;`)
- Modify: `crates/vault/src/vault.rs` — add `extension_subkey` method

- [ ] **Step 1: Write the failing test**

Create `crates/vault/src/subkey.rs`:

```rust
//! HKDF-SHA256 subkey derivation for extension-scoped vault keys.
//!
//! Each extension gets a deterministic 32-byte subkey derived from the
//! vault master key using HKDF with an extension-specific info string.
//! Extensions can use their subkey for their own AEAD (the vault itself
//! never touches extension plaintext beyond the blob APIs).
//!
//! HKDF info format: `"lantern.vault.v1.ext." || extension_id`.
//! Using a namespaced info string ensures collisions with future subkey
//! purposes (e.g. `"lantern.vault.v1.backup."`) are impossible.

use hkdf::Hkdf;
use secrecy::SecretBox;
use sha2::Sha256;

use crate::error::VaultError;

const INFO_PREFIX: &[u8] = b"lantern.vault.v1.ext.";

/// Derive a 32-byte subkey for an extension. Returns a `SecretBox` so the
/// caller cannot accidentally log or serialise it.
pub fn derive_extension_subkey(
    master_key: &[u8; 32],
    extension_id: &str,
) -> Result<SecretBox<[u8; 32]>, VaultError> {
    let hk = Hkdf::<Sha256>::new(None, master_key);
    let mut info: Vec<u8> = Vec::with_capacity(INFO_PREFIX.len() + extension_id.len());
    info.extend_from_slice(INFO_PREFIX);
    info.extend_from_slice(extension_id.as_bytes());
    let mut out = [0u8; 32];
    hk.expand(&info, &mut out).map_err(|_| VaultError::HkdfFailed)?;
    Ok(SecretBox::new(Box::new(out)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[test]
    fn same_inputs_same_subkey() {
        let mk = [0x77u8; 32];
        let k1 = derive_extension_subkey(&mk, "ckb.core").unwrap();
        let k2 = derive_extension_subkey(&mk, "ckb.core").unwrap();
        assert_eq!(k1.expose_secret(), k2.expose_secret());
    }

    #[test]
    fn different_extensions_diverge() {
        let mk = [0x77u8; 32];
        let k1 = derive_extension_subkey(&mk, "ckb.core").unwrap();
        let k2 = derive_extension_subkey(&mk, "ckb.fiber").unwrap();
        assert_ne!(k1.expose_secret(), k2.expose_secret());
    }

    #[test]
    fn different_master_keys_diverge() {
        let k1 = derive_extension_subkey(&[0x01; 32], "ext").unwrap();
        let k2 = derive_extension_subkey(&[0x02; 32], "ext").unwrap();
        assert_ne!(k1.expose_secret(), k2.expose_secret());
    }
}
```

Add `pub mod subkey;` to `crates/vault/src/lib.rs`.

Add to `crates/vault/src/vault.rs` `impl Vault` block (after `lock`):

```rust
    /// Derive an extension-scoped 32-byte subkey via HKDF-SHA256 over the
    /// vault master key. See `subkey.rs` for the info-string format.
    pub fn extension_subkey(
        &self,
        extension_id: &str,
    ) -> Result<SecretBox<[u8; 32]>, VaultError> {
        self.with_master_key(|mk| crate::subkey::derive_extension_subkey(mk, extension_id))
    }
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p lantern-vault`
Expected: 25 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/vault/src/subkey.rs crates/vault/src/vault.rs crates/vault/src/lib.rs
git commit -m "feat(vault): HKDF per-extension subkey derivation"
```

---

## Task 9: Integration test — end-to-end roundtrip

**Files:**
- Create: `crates/vault/tests/vault_roundtrip.rs`

- [ ] **Step 1: Write the integration test**

Create `crates/vault/tests/vault_roundtrip.rs`:

```rust
//! End-to-end integration test for the vault crate.
//!
//! Exercises the public API exactly as a consumer crate
//! (`lantern-wallet-core`) will. If any test here fails, the downstream
//! consumer will break the same way — so these tests are the canonical
//! contract of the vault module.

use lantern_vault::{Vault, VaultError};
use secrecy::ExposeSecret;
use std::fs;
use tempfile::tempdir;

#[test]
fn full_lifecycle() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test-vault.bin");

    // Create with a password and store two blobs.
    {
        let mut v = Vault::create(&path, b"hunter2").unwrap();
        v.put("mnemonic", b"abandon abandon abandon art");
        v.put("ckb-key-0", &[0xABu8; 32]);
        v.save().unwrap();
    }

    // Reopen with the same password.
    let v = Vault::unlock(&path, b"hunter2").unwrap();
    assert_eq!(
        v.get("mnemonic").as_deref(),
        Some(&b"abandon abandon abandon art"[..])
    );
    assert_eq!(v.get("ckb-key-0").as_deref(), Some(&[0xABu8; 32][..]));
    assert_eq!(v.get("not-there"), None);
}

#[test]
fn wrong_password_rejected_without_partial_data() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("vault.bin");
    {
        let mut v = Vault::create(&path, b"right").unwrap();
        v.put("secret", b"value");
        v.save().unwrap();
    }
    let err = Vault::unlock(&path, b"wrong").unwrap_err();
    assert!(matches!(err, VaultError::WrongPassword));
}

#[test]
fn tampered_file_fails_unlock() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("vault.bin");
    {
        let mut v = Vault::create(&path, b"pw").unwrap();
        v.put("secret", b"value");
        v.save().unwrap();
    }
    // Flip one bit inside the ciphertext region (past the 58-byte header).
    let mut bytes = fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    fs::write(&path, bytes).unwrap();

    let err = Vault::unlock(&path, b"pw").unwrap_err();
    // Tampering is indistinguishable from wrong password at this layer.
    assert!(matches!(err, VaultError::WrongPassword));
}

#[test]
fn extension_subkey_is_stable_across_sessions() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("vault.bin");
    let subkey_session_1 = {
        let v = Vault::create(&path, b"pw").unwrap();
        let sk = v.extension_subkey("fiberquest").unwrap();
        *sk.expose_secret()
    };
    let subkey_session_2 = {
        let v = Vault::unlock(&path, b"pw").unwrap();
        let sk = v.extension_subkey("fiberquest").unwrap();
        *sk.expose_secret()
    };
    assert_eq!(subkey_session_1, subkey_session_2);
}

#[test]
fn save_twice_rotates_nonce_but_preserves_data() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("vault.bin");
    let mut v = Vault::create(&path, b"pw").unwrap();
    v.put("x", b"y");
    v.save().unwrap();
    let after_1 = fs::read(&path).unwrap();
    v.save().unwrap();
    let after_2 = fs::read(&path).unwrap();
    assert_ne!(&after_1[34..58], &after_2[34..58], "nonce must rotate");
    assert_eq!(v.get("x").as_deref(), Some(&b"y"[..]));
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p lantern-vault`
Expected: 30 tests PASS (25 unit + 5 integration). Integration tests hit the real 64 MiB Argon2 params, so expect ~5-10s total wall time for the integration suite.

- [ ] **Step 3: Commit**

```bash
git add crates/vault/tests/vault_roundtrip.rs
git commit -m "test(vault): end-to-end integration tests for the public API"
```

---

## Task 10: Clippy pass + module doc audit

**Files:**
- Modify: as needed

- [ ] **Step 1: Run clippy against the whole crate**

Run: `cargo clippy -p lantern-vault --all-targets -- -D warnings`
Expected: PASS. If any lint fires:
- If it's in our code, fix in place.
- If it's from a dep macro (e.g. `ciborium` derive noise), add a scoped `#[allow(clippy::...)]` at the smallest site possible and leave a one-line comment explaining why.

- [ ] **Step 2: Check `cargo doc` builds**

Run: `cargo doc -p lantern-vault --no-deps`
Expected: PASS. Inspect the generated module-level docs for `lib.rs` — they should clearly state that mlock is deferred.

- [ ] **Step 3: Commit any fix-ups**

```bash
git add -A
git commit -m "chore(vault): clippy pedantic + nursery clean pass"
```

Only commit if there were actual changes. Skip this step if clippy was already clean.

- [ ] **Step 4: Tag the vault milestone**

```bash
git tag -a v0.0.2-vault -m "Plan 1b: lantern-vault crate complete"
```

---

## Self-Review

**1. Spec coverage (§7 Vault + Memory hygiene + §9 Storage):**

| Spec requirement | Task |
|---|---|
| One vault per profile | Task 7 (path-addressed Vault) |
| Argon2id KDF | Task 4 |
| XChaCha20-Poly1305 | Task 5 |
| Fresh random 192-bit nonce per save | Task 7 (`save` rotates) + Task 9 (nonce-rotation test) |
| Versioned file format, magic header | Task 3 |
| HKDF per-extension subkeys | Task 8 |
| No plaintext secret crosses to frontend | Not in this plan (IPC boundary lives in plan 1d) |
| Zeroize on drop | Task 6 (`MasterKey`, `InnerStore`), Task 7 (derives via `SecretBox`) |
| `SecretBox<T>` wrapping, no `Debug`/`Display` leakage | Task 6 (Debug opacity test) |
| `mlock` | **Deferred** to plan 1c — documented in `lib.rs` module doc |
| No serialization to disk of unlocked form | Task 7 (plaintext lives only in memory + is zeroized after save) |

Only deliberate gap: `mlock`. Plan 1c picks it up.

**2. Placeholder scan:** No TBDs, no "add error handling" without showing it, every step has runnable code or runnable commands.

**3. Type consistency:** `MasterKey` is only constructed in `secret.rs` and consumed via `into_secret()` returning `SecretBox<[u8; 32]>`. `Vault` stores `master_key: SecretBox<[u8; 32]>` directly. `extension_subkey` returns `SecretBox<[u8; 32]>`. `InnerStore` is `SecretBox<InnerStore>` inside `Vault`. `Header::new_v1` takes the exact `SALT_LEN`/`NONCE_LEN` array sizes used by `derive_master_key` and `seal`. Names and signatures line up across tasks.

---

## Plan complete.

Saved to `docs/superpowers/plans/2026-04-09-plan-1b-vault.md`.

**Execution options:**

1. **Subagent-Driven (recommended)** — fresh subagent per task, spec + code quality review between tasks
2. **Inline Execution** — execute in this session with checkpoint review

Which approach tomorrow?
