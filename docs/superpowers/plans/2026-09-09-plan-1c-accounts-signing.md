# Lantern Plan 1c — Accounts, secp256k1 Signing, SDK Schema, mlock — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** After this plan a Lantern profile can create or import a BIP39 wallet (standard or Quantum Purse combined phrase), derive Neuron-compatible secp256k1 accounts, list them as public records with CKB addresses, and sign a 32-byte digest through one `LockModule`-dispatched entry point, with the seed in the vault and page-locked unlocked state.

**Architecture:** Four crates in a strict downward dependency order: `wallet-core` → {`vault`, `account-registry`, `signer-secp256k1`} → `sdk-schema`. `sdk-schema` owns the IPC types and the `LockModule` trait. `signer-secp256k1` is stateless curve math plus one `Secp256k1Lock` implementor. `account-registry` is a plaintext, public-only JSON store beside the vault. `wallet-core` owns the seed lifecycle (`Keyring`), the module map (`LockRegistry`), and the single signing seam (`SigningCoordinator`).

**Tech Stack:** Rust 1.92 / edition 2024, `secp256k1` 0.33 (libsecp256k1, `recovery`), `ckb-hash` 1.1, `hmac` 0.12 + `sha2` 0.10 (hand-rolled BIP32), `bip39` 2.2 (`rand`, `zeroize`), `pbkdf2` 0.12, `bech32` 0.12, `region` 4.0, `specta` =2.0.0-rc.20 + `specta-typescript` 0.0.7, `secrecy` 0.10, `zeroize` 1.8, `serde_json`, `hex` 0.4.

**Spec:** `docs/superpowers/specs/2026-09-09-plan-1c-accounts-signing-design.md`

## Global Constraints

- Workspace lints apply to every crate: `unsafe_code = "forbid"`, clippy `all + pedantic + nursery` at warn, and CI runs `cargo clippy --workspace --all-targets -- -D warnings`. Test code must be clippy-clean too. Allowed: `module_name_repetitions`, `must_use_candidate`, `missing_errors_doc`, `missing_panics_doc`.
- Pedantic `doc_markdown` is on: put backticks around every identifier or CamelCase word in a doc comment.
- Nursery `missing_const_for_fn` is on: mark a function `const fn` whenever the body allows it.
- Nursery `use_self` is on: write `Self` inside `impl` blocks.
- Every crate keeps `#![forbid(unsafe_code)]` at the top of `lib.rs`.
- No `Debug`, `Display`, `Clone`, or `Serialize` on any secret-bearing type (`SigningKey`, `Phrase`, seeds, entropy).
- Error `Display` text never contains key bytes, phrase words, or file contents.
- Every cryptographic assertion uses an external oracle vector quoted in this plan. Never pin a test to the implementation's own output.
- Commit after every task with the `<type>: <description>` format from `~/.claude/rules/git-workflow.md`.
- Run `cargo fmt --all` before each commit.
- Use `~/.claude/rules/ckb-transactions.md` for any CKB byte-layout question; append a feedback line to `ckb-transactions.feedback.md` at the end of the plan.

## File Structure

```
Cargo.toml                                   # workspace deps + version 0.0.3 (Task 1)
.github/workflows/ci.yml                     # vault no-default-features run (Task 10)
crates/sdk-schema/src/
  lib.rs                                     # re-exports (Task 2)
  types.rs                                   # Network, LockType, AccountCapabilities, Derivation, AccountRecord (Task 2)
  lock.rs                                    # ScriptTemplate, SeedKind, LockModule trait (Task 2)
  error.rs                                   # SchemaError, LockError (Task 2)
  export.rs                                  # typescript_bindings() (Task 2)
crates/signer-secp256k1/src/
  lib.rs                                     # re-exports (Task 3)
  error.rs                                   # SignerError (Task 3)
  key.rs                                     # SigningKey, PublicKey, public_key() (Task 3)
  hash.rs                                    # blake160() (Task 3)
  sign.rs                                    # sign_recoverable(), recover() (Task 4)
  hd.rs                                      # BIP32 CKD, derive_ckb_key() (Task 5)
  sighash.rs                                 # sighash_all() (Task 6)
  lock.rs                                    # Secp256k1Lock: LockModule (Task 7)
crates/account-registry/src/
  lib.rs                                     # re-exports (Task 8)
  error.rs                                   # RegistryError (Task 8)
  address.rs                                 # encode_full() (Task 8)
  store.rs                                   # StoredAccount, AccountRegistry, account_id() (Task 9)
  record.rs                                  # to_record() (Task 9)
crates/vault/src/
  mlock.rs                                   # PageLocks (Task 10)
  vault.rs                                   # get() -> SecretSlice, locks field (Task 10)
  lib.rs                                     # module doc update (Task 10)
crates/wallet-core/src/
  lib.rs                                     # re-exports (Task 11)
  error.rs                                   # CoreError (Task 11)
  mnemonic.rs                                # WordCount, MnemonicFormat, Phrase, parse/render/bip39_seed (Task 11)
  keyring.rs                                 # Keyring (Task 12)
  locks.rs                                   # LockRegistry (Task 12)
  core.rs                                    # ProfilePaths, WalletCore, SigningCoordinator (Task 13)
crates/wallet-core/tests/wallet_e2e.rs       # end-to-end (Task 14)
```

## Oracle vectors used in this plan

Every vector below has an external source. Copy them exactly.

| Name | Source | Value |
|---|---|---|
| BIP32 TV1 seed | BIP-0032 test vector 1 | `000102030405060708090a0b0c0d0e0f` |
| TV1 `m` | decoded from the xprv in BIP-0032 | key `e8f32e723decf4051aefac8e2c93c9c5b214313817cdb01a1494b917c8436b35`, chain `873dff81c02f525623fd1fe5167eac3a55a049de3d314bb42ee227ffed37d508` |
| TV1 `m/0'` | same | key `edb2e14f9ee77d26dd93b4ecede8d16ed408ce149b6cd80b0715a2d911a0afea`, chain `47fdacbd0f1097043b78c63c20c34ef4ed9a111d980047ad16282c7ae6236141` |
| TV1 `m/0'/1` | same | key `3c6cb8d0f6a264c91ea8b5030fadaa8e538b020f0a387421a12de9319dc93368`, chain `2a7857631386ba23dacac34180dd1983734e444fdbf774041578e9b6adb37c19` |
| TV1 `m/0'/1/2'` | same | key `cbce0d719ecf7431d88e6a89fa1483e02e35092af60c042b1df2ff59fa424dca`, chain `04466b9cc8e161e966409ca52986c584f07e9dc81f735db683c3ff6ec7b1503f` |
| TV1 `m/0'/1/2'/2` | same | key `0f479245fb19a38a1954c5c7c0ebab2f9bdfd96a17563ef28a6a4b1a2a764ef4`, chain `cfb71883f01676f587d023cc53a35bc7f88f724b1f8c2892ac1275ac822a3edd` |
| TV1 `m/0'/1/2'/2/1000000000` | same | key `471b76e389e528d6de6d816857e012c5455051cad6660850e58372a6c3e6e7c8`, chain `c783e67b921d2beb8f6b389cc646d7263b4145701dadd2161548a8b078e65e9e` |
| lumos `m/44'/309'/0'` from TV1 seed | `lumos/packages/hd/tests/keychain.test.ts` | key `bb39d218506b30ca69b0f3112427877d983dd3cd2cabc742ab723e2964d98016`, pub `03e5b310636a0f6e7dcdfffa98f28d7ed70df858bb47acf13db830bfde3510b3f3`, chain `37e85a19f54f0a242a35599abac64a71aacc21e3a5860dd024377ffc7e6827d8` |
| lumos `m/44'/309'/0'/0/0` from TV1 seed | same | key `fcba4708f1f07ddc00fc77422d7a70c72b3456f5fef3b2f68368cdee4e6fb498`, pub `0331b3c0225388c5010e3507beb28ecf409c022ef6f358f02b139cbae082f5a2a3`, chain `c4b7aef857b625bbb0497267ed51151d090f81737f4f22a0ac3673483b927090` |
| blake160 of that pub | Python `hashlib.blake2b(person=b"ckb-default-hash")` | `02e830bd6fe19912ffb7b0b134cbe53178b9e8f1` |
| its addresses | Python bech32m validated against RFC 21 | testnet `ckt1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqgzaqct6mlpnyf0ldasky6vhef30zu73ugvy42t5`, mainnet `ckb1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqgzaqct6mlpnyf0ldasky6vhef30zu73ugzk79pv` |
| lumos "tank planet" mnemonic | same file | phrase `tank planet champion pottery together intact quick police asset flower sudden question`, seed `1371018cfad5990f5e451bf586d59c3820a8671162d8700533549b0df61a63330e5cd5099a5d3938f833d51e4572104868bfac7cfe5b4063b1509a995652bc08`, master key `37d25afe073a6ba17badc2df8e91fc0de59ed88bcad6b9a0c2210f325fafca61`, `m/44'/309'/0'` `2925f5dfcbee3b6ad29100a37ed36cbe92d51069779cc96164182c779c5dc20e`, `m/44'/309'/0'/0` `047fae4f38b3204f93a6b39d6dbcfbf5901f2b09f6afec21cbef6033d01801f1`, `m/44'/309'/0'/0/0` `848422863825f69e66dc7f48a3302459ec845395370c23578817456ad6b04b14` |
| Trezor BIP39 #1 | `lumos/packages/hd/tests/mnemonic/fixtures.json` (empty passphrase) | entropy `00`×16, phrase `abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about`, seed `5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc19a5ac40b389cd370d086206dec8aa6c43daea6690f20ad3d8d48b2d2ce9e38e4` |
| Trezor BIP39 #2 | same | entropy `7f`×16, phrase `legal winner thank year wave sausage worth useful legal winner thank yellow`, seed `878386efb78845b3355bd15ea4d39ef97d179cb712b77d5c12b6be415fffeffe5f377ba02bf3f8544ab800b955e51fbff09828f682052a20faa6addbbddfb096` |
| Trezor BIP39 #3 | same | entropy `80`×16, phrase `letter advice cage absurd amount doctor acoustic avoid letter advice cage above`, seed `77d6be9708c8218738934f84bbbb78a2e048ca007746cb764f0673e4b1812d176bbb173e1a291f31cf633f1d0bad7d3cf071c30e98cd0688b5bcce65ecaceb36` |
| Trezor BIP39 #4 (24 words) | same | entropy `ff`×32, phrase `zoo`×23 then `vote`, seed `e28a37058c7f5112ec9e16a3437cf363a2572d70b6ceb3b6965447623d620f14d06bb321a26b33ec15fcd84a3b5ddfd5520e230c924c87aaa0d559749e044fef` |
| Combined 36-word seed | Python `hashlib.pbkdf2_hmac` over phrases #1+#2+#3 joined by single spaces, salt `mnemonic`, 2048 rounds | `4b4dbd0a319c456707c46f77d6c547267bbb667ecfebd5bf08941ff57ed592e63f7e42b59a38f9b9dd43f9986dd7c776c9805a7cff7a468e20e96a5018661354` |
| RFC 21 full address | `rfcs/0021-ckb-address-format` | code hash `9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8`, hash type `01`, args `b39bbc0b3673c7d36450bc14cfcdad2d559c6c64`, mainnet `ckb1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqdnnw7qkdnnclfkg59uzn8umtfd2kwxceqxwquc4`, testnet (Python bech32m, same payload) `ckt1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqdnnw7qkdnnclfkg59uzn8umtfd2kwxceqgutnjd` |
| CKB blank hash | consensus constant (`ckb-hash::BLANK_HASH`) | `44f4c69744d5f8c55d642062949dcae49bc4e7ef43d388c5a12f42b5633d163e` |
| Sighash oracle tx | CKB testnet block 22356763 via `get_block_by_number` | tx hash `03e1abe59be2f5541d84590222048b4594318fa323e5ab0d377904cb84e624f4`, one input, input lock args `72f72b0cafd31de5072b10e84fc6c9d7d7596db7`, single witness `5500000010000000550000005500000041000000a0d661612ec85fc91e2569cd41d25d7704ed44c0042a79e78dc4dcb8e76fd1a15861db4e81cf318be154818199843007f00085d25e3a778494f9f770a4647a7801` |

The witness above is a `WitnessArgs` molecule table of 85 bytes: 4-byte total (`0x55`), three 4-byte offsets (`0x10`, `0x55`, `0x55`), then `BytesOpt` lock = 4-byte length (`0x41` = 65) followed by the 65-byte signature. Bytes `[20..85]` are the signature. Zeroing them gives the "lock replaced by 65 zero bytes" witness RFC 0019 hashes.

---

### Task 1: Wire workspace dependencies and bump the version

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Modify: `crates/sdk-schema/Cargo.toml`
- Modify: `crates/signer-secp256k1/Cargo.toml`
- Modify: `crates/account-registry/Cargo.toml`
- Modify: `crates/wallet-core/Cargo.toml`
- Modify: `crates/vault/Cargo.toml`

**Interfaces:**
- Produces: every later task's `use` lines resolve. Feature `mlock` on `lantern-vault` (default on).

- [ ] **Step 1: Bump the workspace version**

In `Cargo.toml`, change `version = "0.0.1"` under `[workspace.package]` to `version = "0.0.3"`.

- [ ] **Step 2: Add workspace dependencies**

In `Cargo.toml`, immediately after the `ciborium = "0.2"` line inside `[workspace.dependencies]`, add:

```toml

# Keys + signing (plan 1c)
secp256k1 = { version = "0.33", features = ["recovery"] }
hmac = "0.12"
ckb-hash = "1.1"
bip39 = { version = "2.2", features = ["rand", "zeroize"] }
pbkdf2 = { version = "0.12", features = ["hmac"] }
bech32 = "0.12"
hex = { version = "0.4", features = ["serde"] }
region = "4.0"
```

- [ ] **Step 3: Replace `crates/sdk-schema/Cargo.toml`**

```toml
[package]
name = "lantern-sdk-schema"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
description = "Lantern SDK schemas — shared types exported to TS via tauri-specta"

[lints]
workspace = true

[dependencies]
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true
specta = { workspace = true, features = ["derive", "serde", "serde_json"] }
specta-typescript.workspace = true
thiserror.workspace = true
```

- [ ] **Step 4: Replace `crates/signer-secp256k1/Cargo.toml`**

```toml
[package]
name = "lantern-signer-secp256k1"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
description = "Lantern secp256k1_blake160 signer — canonical CKB sighash signing"

[lints]
workspace = true

[dependencies]
lantern-sdk-schema = { path = "../sdk-schema" }
secp256k1.workspace = true
hmac.workspace = true
sha2.workspace = true
ckb-hash.workspace = true
zeroize.workspace = true
thiserror.workspace = true

[dev-dependencies]
hex.workspace = true
```

- [ ] **Step 5: Replace `crates/account-registry/Cargo.toml`**

```toml
[package]
name = "lantern-account-registry"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
description = "Lantern account registry — public AccountRecord storage"

[lints]
workspace = true

[dependencies]
lantern-sdk-schema = { path = "../sdk-schema" }
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true
bech32.workspace = true
hex.workspace = true
thiserror.workspace = true

[dev-dependencies]
tempfile = "3.10"
```

- [ ] **Step 6: Replace `crates/wallet-core/Cargo.toml`**

```toml
[package]
name = "lantern-wallet-core"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
description = "Lantern wallet core — orchestration, signing coordinator, extension host glue"

[lints]
workspace = true

[dependencies]
lantern-sdk-schema = { path = "../sdk-schema" }
lantern-vault = { path = "../vault" }
lantern-account-registry = { path = "../account-registry" }
lantern-signer-secp256k1 = { path = "../signer-secp256k1" }
bip39.workspace = true
pbkdf2.workspace = true
sha2.workspace = true
rand.workspace = true
secrecy.workspace = true
zeroize.workspace = true
thiserror.workspace = true
tracing.workspace = true

[dev-dependencies]
tempfile = "3.10"
hex.workspace = true
```

- [ ] **Step 7: Add the `mlock` feature to `crates/vault/Cargo.toml`**

Append after the `serde = { workspace = true, features = ["derive"] }` line inside `[dependencies]`:

```toml
region = { workspace = true, optional = true }
```

Then add before `[dev-dependencies]`:

```toml
[features]
default = ["mlock"]
mlock = ["dep:region"]
```

- [ ] **Step 8: Verify the workspace resolves and builds**

Run: `cargo check --workspace`
Expected: success. `blake2b-rs` and `secp256k1-sys` compile C; both need only `cc`, which the Tauri toolchain already provides. Warnings about unused dependencies do not appear (Cargo does not lint those).

Run: `cargo test --workspace`
Expected: the existing 34 vault tests pass, placeholder tests pass.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml Cargo.lock crates/sdk-schema/Cargo.toml crates/signer-secp256k1/Cargo.toml crates/account-registry/Cargo.toml crates/wallet-core/Cargo.toml crates/vault/Cargo.toml
git commit -m "chore: wire plan 1c dependencies, bump workspace to 0.0.3"
```

---

### Task 2: sdk-schema types, `LockModule` trait, TypeScript export

**Files:**
- Create: `crates/sdk-schema/src/types.rs`
- Create: `crates/sdk-schema/src/lock.rs`
- Create: `crates/sdk-schema/src/error.rs`
- Create: `crates/sdk-schema/src/export.rs`
- Replace: `crates/sdk-schema/src/lib.rs`

**Interfaces:**
- Produces:
  - `Network { Mainnet, Testnet }` with `const fn address_prefix(self) -> &'static str`
  - `LockType { Secp256k1Blake160 }` with `const fn slug(self) -> &'static str` returning `"secp256k1_blake160"`
  - `AccountCapabilities { can_sign: bool, hardware: bool }`
  - `Derivation { change: u32, index: u32 }`
  - `AccountRecord { id, label, lock_type, extension_id, address, public_metadata: serde_json::Value, capabilities }`
  - `ScriptTemplate { code_hash: [u8; 32], hash_type: u8 }`, `SeedKind { Bip39Seed, RawEntropy }`
  - `trait LockModule: Send + Sync` (eight methods, below)
  - `SchemaError::Export(String)`, `LockError::{InvalidSeed, InvalidDerivation, Signing(String)}`
  - `fn typescript_bindings() -> Result<String, SchemaError>`

- [ ] **Step 1: Write the failing export test**

Create `crates/sdk-schema/src/export.rs`:

```rust
//! Standalone TypeScript export of every schema type.
//!
//! Plan 1f replaces this with the `tauri-specta` builder, which also emits
//! command wrappers. Until then this function proves the Rust-first schema
//! pipeline from spec §9 produces the wire shapes we expect.

use specta_typescript::Typescript;

use crate::error::SchemaError;
use crate::types::{AccountCapabilities, AccountRecord, Derivation, LockType, Network};

/// Render every IPC type as TypeScript.
pub fn typescript_bindings() -> Result<String, SchemaError> {
    let conf = Typescript::default();
    let chunks = [
        specta_typescript::export::<Network>(&conf),
        specta_typescript::export::<LockType>(&conf),
        specta_typescript::export::<AccountCapabilities>(&conf),
        specta_typescript::export::<Derivation>(&conf),
        specta_typescript::export::<AccountRecord>(&conf),
    ];
    let mut out = String::new();
    for chunk in chunks {
        let rendered = chunk.map_err(|e| SchemaError::Export(e.to_string()))?;
        out.push_str(&rendered);
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::typescript_bindings;

    #[test]
    fn bindings_use_camel_case_and_wire_literals() {
        let ts = typescript_bindings().expect("export succeeds");
        for needle in [
            "AccountRecord",
            "Derivation",
            "lockType",
            "extensionId",
            "publicMetadata",
            "canSign",
            "\"secp256k1_blake160\"",
            "\"mainnet\"",
            "\"testnet\"",
        ] {
            assert!(ts.contains(needle), "missing {needle} in:\n{ts}");
        }
        assert!(!ts.contains("lock_type"), "snake_case leaked into TS:\n{ts}");
        assert!(!ts.contains("can_sign"), "snake_case leaked into TS:\n{ts}");
    }
}
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test -p lantern-sdk-schema`
Expected: compile error, `crate::error` and `crate::types` do not exist.

- [ ] **Step 3: Write `crates/sdk-schema/src/error.rs`**

```rust
//! Error types for the schema crate and the `LockModule` contract.
//!
//! `Display` text must never carry key material. `LockError::Signing`
//! carries a scheme-specific message that implementors must keep free of
//! secrets.

use thiserror::Error;

/// Failure while rendering TypeScript bindings.
#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("TypeScript export failed: {0}")]
    Export(String),
}

/// Failure inside a `LockModule` implementation.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LockError {
    #[error("seed material has the wrong shape for this lock module")]
    InvalidSeed,

    #[error("derivation is out of range for this lock module")]
    InvalidDerivation,

    #[error("signing failed: {0}")]
    Signing(String),
}
```

- [ ] **Step 4: Write `crates/sdk-schema/src/types.rs`**

```rust
//! IPC types. Every type here crosses the Tauri boundary, so each derives
//! `specta::Type` and uses camelCase on the wire.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Which CKB network an address is rendered for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    Mainnet,
    Testnet,
}

impl Network {
    /// Bech32m human-readable prefix.
    pub const fn address_prefix(self) -> &'static str {
        match self {
            Self::Mainnet => "ckb",
            Self::Testnet => "ckt",
        }
    }
}

/// Lock script families Lantern knows how to sign for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum LockType {
    Secp256k1Blake160,
}

impl LockType {
    /// Stable identifier used in account ids and on the wire.
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Secp256k1Blake160 => "secp256k1_blake160",
        }
    }
}

/// What an account can do. Public data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AccountCapabilities {
    pub can_sign: bool,
    pub hardware: bool,
}

/// Position of an account under its lock module's key tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Derivation {
    pub change: u32,
    pub index: u32,
}

/// Public projection of an account. Holds no secret material and is safe
/// to send to any frontend or extension with `accounts.read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AccountRecord {
    pub id: String,
    pub label: String,
    pub lock_type: LockType,
    pub extension_id: String,
    pub address: String,
    pub public_metadata: serde_json::Value,
    pub capabilities: AccountCapabilities,
}

#[cfg(test)]
mod tests {
    use super::{AccountCapabilities, AccountRecord, LockType, Network};

    #[test]
    fn network_prefixes() {
        assert_eq!(Network::Mainnet.address_prefix(), "ckb");
        assert_eq!(Network::Testnet.address_prefix(), "ckt");
    }

    #[test]
    fn lock_type_serialises_as_slug() {
        let json = serde_json::to_string(&LockType::Secp256k1Blake160).expect("serialises");
        assert_eq!(json, "\"secp256k1_blake160\"");
        assert_eq!(LockType::Secp256k1Blake160.slug(), "secp256k1_blake160");
    }

    #[test]
    fn account_record_round_trips_camel_case() {
        let record = AccountRecord {
            id: "secp256k1_blake160-00".into(),
            label: "Main".into(),
            lock_type: LockType::Secp256k1Blake160,
            extension_id: "core.secp256k1".into(),
            address: "ckt1...".into(),
            public_metadata: serde_json::json!({ "lockArgs": "0x00" }),
            capabilities: AccountCapabilities {
                can_sign: true,
                hardware: false,
            },
        };
        let json = serde_json::to_string(&record).expect("serialises");
        assert!(json.contains("\"lockType\":\"secp256k1_blake160\""), "{json}");
        assert!(json.contains("\"canSign\":true"), "{json}");
        let back: AccountRecord = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back, record);
    }
}
```

- [ ] **Step 5: Write `crates/sdk-schema/src/lock.rs`**

```rust
//! The lock-module contract. Every first-party and third-party signer
//! implements `LockModule`; `wallet-core` dispatches through it and never
//! names a concrete scheme.

use crate::error::LockError;
use crate::types::{AccountCapabilities, Derivation, LockType};

/// The script a lock module's accounts are locked by. `hash_type` follows
/// the CKB `ScriptHashType` encoding (`0x00` data, `0x01` type, `0x02`
/// data1, `0x04` data2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptTemplate {
    pub code_hash: [u8; 32],
    pub hash_type: u8,
}

/// Which master secret a module derives from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedKind {
    /// The 64-byte BIP39 seed (PBKDF2 over the phrase). Used by BIP32 schemes.
    Bip39Seed,
    /// The raw mnemonic entropy (16 to 96 bytes). Used by hash-based
    /// post-quantum schemes that HKDF over it, as Quantum Purse does.
    RawEntropy,
}

/// Object-safe contract for a lock family.
///
/// `seed` is a borrowed slice of whatever `seed_kind` asked for. Implementors
/// must not retain it. `sign_digest` returns the bytes destined for the
/// witness lock field, whatever size the scheme needs.
pub trait LockModule: Send + Sync {
    fn lock_type(&self) -> LockType;
    fn extension_id(&self) -> &'static str;
    fn capabilities(&self) -> AccountCapabilities;
    fn script_template(&self) -> ScriptTemplate;
    fn seed_kind(&self) -> SeedKind;
    /// Size of the witness lock placeholder for fee estimation.
    fn witness_lock_len(&self) -> usize;
    fn derive_lock_args(&self, seed: &[u8], derivation: &Derivation) -> Result<Vec<u8>, LockError>;
    fn sign_digest(
        &self,
        seed: &[u8],
        derivation: &Derivation,
        digest: &[u8; 32],
    ) -> Result<Vec<u8>, LockError>;
}

#[cfg(test)]
mod tests {
    use super::{LockModule, ScriptTemplate, SeedKind};
    use crate::error::LockError;
    use crate::types::{AccountCapabilities, Derivation, LockType};

    struct Fake;

    impl LockModule for Fake {
        fn lock_type(&self) -> LockType {
            LockType::Secp256k1Blake160
        }
        fn extension_id(&self) -> &'static str {
            "test.fake"
        }
        fn capabilities(&self) -> AccountCapabilities {
            AccountCapabilities {
                can_sign: true,
                hardware: false,
            }
        }
        fn script_template(&self) -> ScriptTemplate {
            ScriptTemplate {
                code_hash: [0; 32],
                hash_type: 1,
            }
        }
        fn seed_kind(&self) -> SeedKind {
            SeedKind::RawEntropy
        }
        fn witness_lock_len(&self) -> usize {
            1
        }
        fn derive_lock_args(&self, seed: &[u8], _: &Derivation) -> Result<Vec<u8>, LockError> {
            Ok(seed.to_vec())
        }
        fn sign_digest(&self, _: &[u8], _: &Derivation, d: &[u8; 32]) -> Result<Vec<u8>, LockError> {
            Ok(d.to_vec())
        }
    }

    #[test]
    fn trait_is_object_safe() {
        let module: Box<dyn LockModule> = Box::new(Fake);
        let derivation = Derivation {
            change: 0,
            index: 0,
        };
        assert_eq!(module.derive_lock_args(&[1, 2], &derivation), Ok(vec![1, 2]));
        assert_eq!(module.seed_kind(), SeedKind::RawEntropy);
    }
}
```

- [ ] **Step 6: Replace `crates/sdk-schema/src/lib.rs`**

```rust
#![forbid(unsafe_code)]

//! Lantern SDK schemas.
//!
//! Single source of truth for types that cross the IPC boundary into the
//! TypeScript SDK, plus the `LockModule` contract every signer implements.
//! All wire types derive `specta::Type` so `tauri-specta` can export them.

pub mod error;
pub mod export;
pub mod lock;
pub mod types;

pub use error::{LockError, SchemaError};
pub use export::typescript_bindings;
pub use lock::{LockModule, ScriptTemplate, SeedKind};
pub use types::{AccountCapabilities, AccountRecord, Derivation, LockType, Network};
```

- [ ] **Step 7: Run the tests**

Run: `cargo test -p lantern-sdk-schema`
Expected: 5 tests pass.

If `bindings_use_camel_case_and_wire_literals` fails because the TS still shows `lock_type`, the specta derive did not pick up the serde attribute in this pre-release. Add `#[specta(rename_all = "camelCase")]` directly under each `#[serde(rename_all = "camelCase")]` (and `#[specta(rename_all = "snake_case")]` / `"lowercase"` on the enums) and re-run. Both attributes must stay, so serde and specta agree.

- [ ] **Step 8: Clippy and commit**

Run: `cargo clippy -p lantern-sdk-schema --all-targets -- -D warnings`
Expected: clean.

```bash
cargo fmt --all
git add crates/sdk-schema
git commit -m "feat(sdk-schema): IPC types, LockModule trait, TypeScript export"
```

---

### Task 3: signer — `SigningKey`, `PublicKey`, `public_key`, `blake160`

**Files:**
- Create: `crates/signer-secp256k1/src/error.rs`
- Create: `crates/signer-secp256k1/src/key.rs`
- Create: `crates/signer-secp256k1/src/hash.rs`
- Replace: `crates/signer-secp256k1/src/lib.rs`

**Interfaces:**
- Produces:
  - `SignerError::{InvalidKey, InvalidSignature, InvalidSeedLength, DerivationOverflow}`
  - `SigningKey` (zeroize on drop, no Debug/Clone): `fn from_bytes(bytes: [u8; 32]) -> Result<SigningKey, SignerError>`; crate-internal `fn to_secp(&self) -> secp256k1::SecretKey`
  - `PublicKey` (Copy, Debug): `const fn as_bytes(&self) -> &[u8; 33]`, `fn from_bytes([u8; 33]) -> Result<PublicKey, SignerError>`, crate-internal `fn from_secp(&secp256k1::PublicKey) -> PublicKey`
  - `fn public_key(key: &SigningKey) -> PublicKey`
  - `fn blake160(pubkey: &PublicKey) -> [u8; 20]`

Module layout note: modules are private (`mod key;`) with `pub` items re-exported from `lib.rs`. Do not write `pub(crate)` on items inside them; the nursery lint `redundant_pub_crate` rejects it.

- [ ] **Step 1: Write the failing tests**

Create `crates/signer-secp256k1/src/key.rs` with only the test module first:

```rust
//! Key types. `SigningKey` is the only secret-bearing type in this crate;
//! it zeroizes on drop and has no `Debug`, `Clone`, or `Serialize`.
//!
//! Known residue: libsecp256k1's `SecretKey` is `Copy` and does not zeroize
//! on drop, so every call that needs one creates a transient copy on the
//! stack and calls `non_secure_erase` on it before returning. Copies made
//! by the compiler when passing arrays by value are outside our control.

#[cfg(test)]
mod tests {
    use super::{PublicKey, SigningKey, public_key};
    use crate::error::SignerError;

    fn h32(s: &str) -> [u8; 32] {
        let v = hex::decode(s).expect("hex");
        let mut out = [0u8; 32];
        out.copy_from_slice(&v);
        out
    }

    #[test]
    fn public_key_matches_lumos_vector() {
        // lumos keychain.test.ts: m/44'/309'/0'/0/0 from the BIP32 TV1 seed
        let key = SigningKey::from_bytes(h32(
            "fcba4708f1f07ddc00fc77422d7a70c72b3456f5fef3b2f68368cdee4e6fb498",
        ))
        .expect("valid key");
        let pk = public_key(&key);
        assert_eq!(
            hex::encode(pk.as_bytes()),
            "0331b3c0225388c5010e3507beb28ecf409c022ef6f358f02b139cbae082f5a2a3"
        );
    }

    #[test]
    fn zero_and_overflow_keys_are_rejected() {
        assert_eq!(
            SigningKey::from_bytes([0u8; 32]).err(),
            Some(SignerError::InvalidKey)
        );
        assert_eq!(
            SigningKey::from_bytes([0xffu8; 32]).err(),
            Some(SignerError::InvalidKey)
        );
    }

    #[test]
    fn public_key_from_bytes_validates_point() {
        let good = hex::decode("0331b3c0225388c5010e3507beb28ecf409c022ef6f358f02b139cbae082f5a2a3")
            .expect("hex");
        let mut arr = [0u8; 33];
        arr.copy_from_slice(&good);
        assert!(PublicKey::from_bytes(arr).is_ok());
        arr[0] = 0x05;
        assert_eq!(PublicKey::from_bytes(arr).err(), Some(SignerError::InvalidKey));
    }
}
```

Create `crates/signer-secp256k1/src/hash.rs` with only its test module:

```rust
//! CKB hashing helpers built on `ckb-hash`, the consensus implementation
//! (blake2b-256, personalisation `ckb-default-hash`).

#[cfg(test)]
mod tests {
    use super::blake160;
    use crate::key::PublicKey;

    #[test]
    fn ckb_hash_personalisation_matches_consensus_blank_hash() {
        // Guards against a mis-personalised blake2b. This constant is the
        // network's hash of the empty string.
        assert_eq!(
            hex::encode(ckb_hash::blake2b_256(b"")),
            "44f4c69744d5f8c55d642062949dcae49bc4e7ef43d388c5a12f42b5633d163e"
        );
    }

    #[test]
    fn blake160_matches_independent_blake2b() {
        // Python: hashlib.blake2b(pub, digest_size=32, person=b"ckb-default-hash").digest()[:20]
        let pk = hex::decode("0331b3c0225388c5010e3507beb28ecf409c022ef6f358f02b139cbae082f5a2a3")
            .expect("hex");
        let mut arr = [0u8; 33];
        arr.copy_from_slice(&pk);
        let pk = PublicKey::from_bytes(arr).expect("valid point");
        assert_eq!(
            hex::encode(blake160(&pk)),
            "02e830bd6fe19912ffb7b0b134cbe53178b9e8f1"
        );
    }
}
```

Replace `crates/signer-secp256k1/src/lib.rs`:

```rust
#![forbid(unsafe_code)]

//! Lantern `secp256k1_blake160` signer.
//!
//! Stateless curve math for the canonical CKB lock: BIP32 derivation on the
//! Neuron-compatible path, recoverable ECDSA over a 32-byte digest, the
//! RFC 0019 `sighash_all` digest, and the `LockModule` implementation that
//! `wallet-core` dispatches to. No storage, no I/O.

mod error;
mod hash;
mod key;

pub use error::SignerError;
pub use hash::blake160;
pub use key::{PublicKey, SigningKey, public_key};
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p lantern-signer-secp256k1`
Expected: compile errors, `error` module missing and `SigningKey` undefined.

- [ ] **Step 3: Write `crates/signer-secp256k1/src/error.rs`**

```rust
//! Error type. `Display` never carries key bytes.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SignerError {
    #[error("invalid secp256k1 secret key")]
    InvalidKey,

    #[error("invalid or unrecoverable signature")]
    InvalidSignature,

    #[error("seed must be 16 to 64 bytes")]
    InvalidSeedLength,

    #[error("derivation index must be below 2^31")]
    DerivationOverflow,
}
```

- [ ] **Step 4: Write the implementation in `key.rs` (above the test module)**

```rust
use secp256k1::{PublicKey as SecpPublicKey, SecretKey};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::SignerError;

/// A 32-byte secp256k1 secret scalar. Validated at construction.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SigningKey([u8; 32]);

impl SigningKey {
    /// Accepts the bytes by value and wipes the argument slot after copying
    /// into the zeroizing wrapper.
    pub fn from_bytes(mut bytes: [u8; 32]) -> Result<Self, SignerError> {
        let mut probe = SecretKey::from_byte_array(bytes).map_err(|_| SignerError::InvalidKey)?;
        probe.non_secure_erase();
        let key = Self(bytes);
        bytes.zeroize();
        Ok(key)
    }

    /// Transient libsecp256k1 key. Callers must `non_secure_erase` it.
    pub fn to_secp(&self) -> SecretKey {
        SecretKey::from_byte_array(self.0).expect("validated at construction")
    }
}

/// A 33-byte compressed SEC1 public key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicKey([u8; 33]);

impl PublicKey {
    pub const fn as_bytes(&self) -> &[u8; 33] {
        &self.0
    }

    pub fn from_bytes(bytes: [u8; 33]) -> Result<Self, SignerError> {
        SecpPublicKey::from_slice(&bytes).map_err(|_| SignerError::InvalidKey)?;
        Ok(Self(bytes))
    }

    pub fn from_secp(pk: &SecpPublicKey) -> Self {
        Self(pk.serialize())
    }
}

/// Compressed public key for a signing key.
pub fn public_key(key: &SigningKey) -> PublicKey {
    let mut sk = key.to_secp();
    let pk = SecpPublicKey::from_secret_key(&sk);
    sk.non_secure_erase();
    PublicKey::from_secp(&pk)
}
```

- [ ] **Step 5: Write the implementation in `hash.rs` (above the test module)**

```rust
use crate::key::PublicKey;

/// First 20 bytes of `blake2b_256(pubkey)`: the lock args of a
/// `secp256k1_blake160` cell.
pub fn blake160(pubkey: &PublicKey) -> [u8; 20] {
    let full = ckb_hash::blake2b_256(pubkey.as_bytes());
    let mut out = [0u8; 20];
    out.copy_from_slice(&full[..20]);
    out
}
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p lantern-signer-secp256k1`
Expected: 5 tests pass.

- [ ] **Step 7: Clippy and commit**

Run: `cargo clippy -p lantern-signer-secp256k1 --all-targets -- -D warnings`
Expected: clean.

```bash
cargo fmt --all
git add crates/signer-secp256k1
git commit -m "feat(signer-secp256k1): SigningKey, PublicKey, blake160 with lumos vectors"
```

---

### Task 4: signer — `sign_recoverable` and `recover`

**Files:**
- Create: `crates/signer-secp256k1/src/sign.rs`
- Modify: `crates/signer-secp256k1/src/lib.rs`

**Interfaces:**
- Consumes: `SigningKey::to_secp`, `PublicKey::from_secp` (Task 3)
- Produces:
  - `fn sign_recoverable(key: &SigningKey, digest: &[u8; 32]) -> [u8; 65]` — `r ‖ s ‖ recid`
  - `fn recover(signature: &[u8; 65], digest: &[u8; 32]) -> Result<PublicKey, SignerError>`

- [ ] **Step 1: Write the failing tests**

Create `crates/signer-secp256k1/src/sign.rs`:

```rust
//! Recoverable ECDSA over a 32-byte digest in the layout the
//! `secp256k1_blake160_sighash_all` lock reads: 64-byte compact signature
//! followed by one recovery-id byte.

#[cfg(test)]
mod tests {
    use super::{recover, sign_recoverable};
    use crate::error::SignerError;
    use crate::key::{SigningKey, public_key};

    fn key() -> SigningKey {
        let v = hex::decode("fcba4708f1f07ddc00fc77422d7a70c72b3456f5fef3b2f68368cdee4e6fb498")
            .expect("hex");
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&v);
        SigningKey::from_bytes(arr).expect("valid")
    }

    #[test]
    fn signature_is_deterministic_and_recovers_the_signer() {
        let key = key();
        let digest = [0x42u8; 32];
        let a = sign_recoverable(&key, &digest);
        let b = sign_recoverable(&key, &digest);
        assert_eq!(a, b, "RFC 6979 nonces make signing deterministic");
        assert!(a[64] <= 3, "recid byte must be 0..=3, got {}", a[64]);
        let recovered = recover(&a, &digest).expect("recovers");
        assert_eq!(recovered, public_key(&key));
    }

    #[test]
    fn wrong_digest_does_not_recover_the_signer() {
        let key = key();
        let sig = sign_recoverable(&key, &[0x42u8; 32]);
        let other = recover(&sig, &[0x43u8; 32]);
        assert!(other.is_none_or(|pk| pk != public_key(&key)));
    }

    #[test]
    fn bad_recovery_id_is_rejected() {
        let key = key();
        let mut sig = sign_recoverable(&key, &[0x42u8; 32]);
        sig[64] = 4;
        assert_eq!(recover(&sig, &[0x42u8; 32]).err(), Some(SignerError::InvalidSignature));
    }
}
```

Add to `lib.rs`: `mod sign;` and `pub use sign::{recover, sign_recoverable};`.

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p lantern-signer-secp256k1`
Expected: compile error, `sign_recoverable` not found.

- [ ] **Step 3: Write the implementation (above the test module in `sign.rs`)**

```rust
use secp256k1::ecdsa::{RecoverableSignature, RecoveryId};
use secp256k1::{Message, Secp256k1};

use crate::error::SignerError;
use crate::key::{PublicKey, SigningKey};

/// Sign a digest. Output layout is `r ‖ s ‖ recid`.
pub fn sign_recoverable(key: &SigningKey, digest: &[u8; 32]) -> [u8; 65] {
    let secp = Secp256k1::new();
    let mut sk = key.to_secp();
    let sig = secp.sign_ecdsa_recoverable(Message::from_digest(*digest), &sk);
    sk.non_secure_erase();
    let (recid, compact) = sig.serialize_compact();
    let mut out = [0u8; 65];
    out[..64].copy_from_slice(&compact);
    out[64] = u8::from(recid);
    out
}

/// Recover the public key that produced `signature` over `digest`.
pub fn recover(signature: &[u8; 65], digest: &[u8; 32]) -> Result<PublicKey, SignerError> {
    let recid = RecoveryId::try_from(i32::from(signature[64]))
        .map_err(|_| SignerError::InvalidSignature)?;
    let sig = RecoverableSignature::from_compact(&signature[..64], recid)
        .map_err(|_| SignerError::InvalidSignature)?;
    let secp = Secp256k1::new();
    let pk = secp
        .recover_ecdsa(Message::from_digest(*digest), &sig)
        .map_err(|_| SignerError::InvalidSignature)?;
    Ok(PublicKey::from_secp(&pk))
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p lantern-signer-secp256k1`
Expected: 8 tests pass.

- [ ] **Step 5: Clippy and commit**

Run: `cargo clippy -p lantern-signer-secp256k1 --all-targets -- -D warnings`
Expected: clean.

```bash
cargo fmt --all
git add crates/signer-secp256k1
git commit -m "feat(signer-secp256k1): recoverable ECDSA sign + recover"
```

---

### Task 5: signer — hand-rolled BIP32 and `derive_ckb_key`

**Files:**
- Create: `crates/signer-secp256k1/src/hd.rs`
- Modify: `crates/signer-secp256k1/src/lib.rs`

**Interfaces:**
- Consumes: `SigningKey::from_bytes` (Task 3)
- Produces:
  - `const CKB_COIN_TYPE: u32 = 309`
  - `enum Branch { External, Internal }` with `const fn index(self) -> u32` (0, 1) and `TryFrom<u32>` (`DerivationOverflow` for other values)
  - `fn derive_ckb_key(seed: &[u8], branch: Branch, index: u32) -> Result<SigningKey, SignerError>` on `m/44'/309'/0'/branch/index`; seed 16..=64 bytes; index < 2^31
  - crate-visible `fn derive_path(seed: &[u8], path: &[u32]) -> Result<ExtendedKey, SignerError>` and `struct ExtendedKey { key: [u8; 32], chain_code: [u8; 32] }` for tests

- [ ] **Step 1: Write the failing tests**

Create `crates/signer-secp256k1/src/hd.rs`:

```rust
//! BIP32 hardened and normal child derivation, written directly on
//! `hmac`/`sha2` and libsecp256k1's `add_tweak` so the crate carries one
//! curve implementation. Verified against BIP-0032 test vector 1 and the
//! lumos `m/44'/309'/0'` vectors.
//!
//! Only private derivation exists; there is no xpub export.

#[cfg(test)]
mod tests {
    use super::{Branch, HARDENED, derive_ckb_key, derive_path};
    use crate::error::SignerError;
    use crate::key::public_key;

    const TV1_SEED: &str = "000102030405060708090a0b0c0d0e0f";
    const TANK_SEED: &str = "1371018cfad5990f5e451bf586d59c3820a8671162d8700533549b0df61a63330e5cd5099a5d3938f833d51e4572104868bfac7cfe5b4063b1509a995652bc08";

    fn seed(s: &str) -> Vec<u8> {
        hex::decode(s).expect("hex")
    }

    #[test]
    fn bip32_test_vector_1_chain() {
        let seed = seed(TV1_SEED);
        let cases: [(&[u32], &str, &str); 6] = [
            (
                &[],
                "e8f32e723decf4051aefac8e2c93c9c5b214313817cdb01a1494b917c8436b35",
                "873dff81c02f525623fd1fe5167eac3a55a049de3d314bb42ee227ffed37d508",
            ),
            (
                &[HARDENED],
                "edb2e14f9ee77d26dd93b4ecede8d16ed408ce149b6cd80b0715a2d911a0afea",
                "47fdacbd0f1097043b78c63c20c34ef4ed9a111d980047ad16282c7ae6236141",
            ),
            (
                &[HARDENED, 1],
                "3c6cb8d0f6a264c91ea8b5030fadaa8e538b020f0a387421a12de9319dc93368",
                "2a7857631386ba23dacac34180dd1983734e444fdbf774041578e9b6adb37c19",
            ),
            (
                &[HARDENED, 1, 2 | HARDENED],
                "cbce0d719ecf7431d88e6a89fa1483e02e35092af60c042b1df2ff59fa424dca",
                "04466b9cc8e161e966409ca52986c584f07e9dc81f735db683c3ff6ec7b1503f",
            ),
            (
                &[HARDENED, 1, 2 | HARDENED, 2],
                "0f479245fb19a38a1954c5c7c0ebab2f9bdfd96a17563ef28a6a4b1a2a764ef4",
                "cfb71883f01676f587d023cc53a35bc7f88f724b1f8c2892ac1275ac822a3edd",
            ),
            (
                &[HARDENED, 1, 2 | HARDENED, 2, 1_000_000_000],
                "471b76e389e528d6de6d816857e012c5455051cad6660850e58372a6c3e6e7c8",
                "c783e67b921d2beb8f6b389cc646d7263b4145701dadd2161548a8b078e65e9e",
            ),
        ];
        for (path, key, chain) in cases {
            let node = derive_path(&seed, path).expect("derives");
            assert_eq!(hex::encode(node.key), key, "key at {path:?}");
            assert_eq!(hex::encode(node.chain_code), chain, "chain at {path:?}");
        }
    }

    #[test]
    fn lumos_ckb_account_node_from_tv1_seed() {
        let node = derive_path(&seed(TV1_SEED), &[44 | HARDENED, 309 | HARDENED, HARDENED])
            .expect("derives");
        assert_eq!(
            hex::encode(node.key),
            "bb39d218506b30ca69b0f3112427877d983dd3cd2cabc742ab723e2964d98016"
        );
        assert_eq!(
            hex::encode(node.chain_code),
            "37e85a19f54f0a242a35599abac64a71aacc21e3a5860dd024377ffc7e6827d8"
        );
    }

    #[test]
    fn lumos_first_receiving_key_from_tv1_seed() {
        let key = derive_ckb_key(&seed(TV1_SEED), Branch::External, 0).expect("derives");
        assert_eq!(
            hex::encode(public_key(&key).as_bytes()),
            "0331b3c0225388c5010e3507beb28ecf409c022ef6f358f02b139cbae082f5a2a3"
        );
        let node = derive_path(&seed(TV1_SEED), &[44 | HARDENED, 309 | HARDENED, HARDENED, 0, 0])
            .expect("derives");
        assert_eq!(
            hex::encode(node.key),
            "fcba4708f1f07ddc00fc77422d7a70c72b3456f5fef3b2f68368cdee4e6fb498"
        );
        assert_eq!(
            hex::encode(node.chain_code),
            "c4b7aef857b625bbb0497267ed51151d090f81737f4f22a0ac3673483b927090"
        );
    }

    #[test]
    fn lumos_tank_planet_seed_chain() {
        let seed = seed(TANK_SEED);
        assert_eq!(seed.len(), 64);
        let m = derive_path(&seed, &[]).expect("derives");
        assert_eq!(
            hex::encode(m.key),
            "37d25afe073a6ba17badc2df8e91fc0de59ed88bcad6b9a0c2210f325fafca61"
        );
        let acct = derive_path(&seed, &[44 | HARDENED, 309 | HARDENED, HARDENED]).expect("derives");
        assert_eq!(
            hex::encode(acct.key),
            "2925f5dfcbee3b6ad29100a37ed36cbe92d51069779cc96164182c779c5dc20e"
        );
        let external = derive_path(&seed, &[44 | HARDENED, 309 | HARDENED, HARDENED, 0]).expect("derives");
        assert_eq!(
            hex::encode(external.key),
            "047fae4f38b3204f93a6b39d6dbcfbf5901f2b09f6afec21cbef6033d01801f1"
        );
        let first = derive_path(&seed, &[44 | HARDENED, 309 | HARDENED, HARDENED, 0, 0]).expect("derives");
        assert_eq!(
            hex::encode(first.key),
            "848422863825f69e66dc7f48a3302459ec845395370c23578817456ad6b04b14"
        );
    }

    #[test]
    fn rejects_bad_seed_length_and_hardened_index() {
        assert_eq!(
            derive_ckb_key(&[0u8; 15], Branch::External, 0).err(),
            Some(SignerError::InvalidSeedLength)
        );
        assert_eq!(
            derive_ckb_key(&seed(TV1_SEED), Branch::External, HARDENED).err(),
            Some(SignerError::DerivationOverflow)
        );
        assert_eq!(Branch::try_from(2).err(), Some(SignerError::DerivationOverflow));
        assert_eq!(Branch::try_from(1), Ok(Branch::Internal));
    }
}
```

Add to `lib.rs`: `mod hd;` and `pub use hd::{Branch, CKB_COIN_TYPE, derive_ckb_key};`.

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p lantern-signer-secp256k1`
Expected: compile error, `derive_path` not found.

- [ ] **Step 3: Write the implementation (above the test module in `hd.rs`)**

```rust
use hmac::{Hmac, Mac};
use secp256k1::{PublicKey as SecpPublicKey, Scalar, SecretKey};
use sha2::Sha512;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::error::SignerError;
use crate::key::SigningKey;

type HmacSha512 = Hmac<Sha512>;

/// Hardened-index bit.
pub const HARDENED: u32 = 0x8000_0000;
/// SLIP-0044 coin type for CKB.
pub const CKB_COIN_TYPE: u32 = 309;
const PURPOSE_BIP44: u32 = 44;

/// BIP44 chain branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Branch {
    /// Receiving addresses (`.../0/i`).
    External,
    /// Change addresses (`.../1/i`).
    Internal,
}

impl Branch {
    pub const fn index(self) -> u32 {
        match self {
            Self::External => 0,
            Self::Internal => 1,
        }
    }
}

impl TryFrom<u32> for Branch {
    type Error = SignerError;

    fn try_from(value: u32) -> Result<Self, SignerError> {
        match value {
            0 => Ok(Self::External),
            1 => Ok(Self::Internal),
            _ => Err(SignerError::DerivationOverflow),
        }
    }
}

/// A private node in the key tree. Zeroized on drop.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct ExtendedKey {
    pub key: [u8; 32],
    pub chain_code: [u8; 32],
}

fn hmac_sha512(key: &[u8], parts: &[&[u8]]) -> Zeroizing<[u8; 64]> {
    let mut mac = HmacSha512::new_from_slice(key).expect("HMAC accepts any key length");
    for part in parts {
        mac.update(part);
    }
    let mut tag = mac.finalize().into_bytes();
    let mut out = Zeroizing::new([0u8; 64]);
    out.copy_from_slice(&tag);
    tag.as_mut_slice().zeroize();
    out
}

fn split(i: &[u8; 64]) -> Result<ExtendedKey, SignerError> {
    let mut node = ExtendedKey {
        key: [0u8; 32],
        chain_code: [0u8; 32],
    };
    node.key.copy_from_slice(&i[..32]);
    node.chain_code.copy_from_slice(&i[32..]);
    let mut probe = SecretKey::from_byte_array(node.key).map_err(|_| SignerError::InvalidKey)?;
    probe.non_secure_erase();
    Ok(node)
}

fn master_from_seed(seed: &[u8]) -> Result<ExtendedKey, SignerError> {
    if !(16..=64).contains(&seed.len()) {
        return Err(SignerError::InvalidSeedLength);
    }
    let i = hmac_sha512(b"Bitcoin seed", &[seed]);
    split(&i)
}

fn derive_child(parent: &ExtendedKey, index: u32) -> Result<ExtendedKey, SignerError> {
    let i = if index & HARDENED == 0 {
        let mut sk = SecretKey::from_byte_array(parent.key).map_err(|_| SignerError::InvalidKey)?;
        let pk = SecpPublicKey::from_secret_key(&sk).serialize();
        sk.non_secure_erase();
        hmac_sha512(&parent.chain_code, &[&pk, &index.to_be_bytes()])
    } else {
        hmac_sha512(&parent.chain_code, &[&[0u8], &parent.key, &index.to_be_bytes()])
    };
    let mut left = [0u8; 32];
    left.copy_from_slice(&i[..32]);
    let tweak = Scalar::from_be_bytes(left).map_err(|_| SignerError::InvalidKey)?;
    left.zeroize();
    let parent_sk = SecretKey::from_byte_array(parent.key).map_err(|_| SignerError::InvalidKey)?;
    let mut child_sk = parent_sk.add_tweak(&tweak).map_err(|_| SignerError::InvalidKey)?;
    let mut child = ExtendedKey {
        key: child_sk.secret_bytes(),
        chain_code: [0u8; 32],
    };
    child_sk.non_secure_erase();
    child.chain_code.copy_from_slice(&i[32..]);
    Ok(child)
}

/// Walk an arbitrary path from the seed. Hardened levels carry the
/// `HARDENED` bit.
pub fn derive_path(seed: &[u8], path: &[u32]) -> Result<ExtendedKey, SignerError> {
    let mut node = master_from_seed(seed)?;
    for &index in path {
        node = derive_child(&node, index)?;
    }
    Ok(node)
}

/// Derive the signing key at `m/44'/309'/0'/branch/index`.
pub fn derive_ckb_key(seed: &[u8], branch: Branch, index: u32) -> Result<SigningKey, SignerError> {
    if index >= HARDENED {
        return Err(SignerError::DerivationOverflow);
    }
    let path = [
        PURPOSE_BIP44 | HARDENED,
        CKB_COIN_TYPE | HARDENED,
        HARDENED,
        branch.index(),
        index,
    ];
    let node = derive_path(seed, &path)?;
    SigningKey::from_bytes(node.key)
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p lantern-signer-secp256k1`
Expected: 13 tests pass. If `bip32_test_vector_1_chain` fails at the first case, the master HMAC key is wrong (`b"Bitcoin seed"` exactly). If it fails at `[HARDENED]`, the hardened data must be `0x00 ‖ key ‖ index` with a big-endian index. If it fails at `[HARDENED, 1]`, the normal-child data must be the 33-byte compressed public key.

- [ ] **Step 5: Clippy and commit**

Run: `cargo clippy -p lantern-signer-secp256k1 --all-targets -- -D warnings`
Expected: clean.

```bash
cargo fmt --all
git add crates/signer-secp256k1
git commit -m "feat(signer-secp256k1): BIP32 derivation on m/44'/309'/0' with BIP32 + lumos vectors"
```

---

### Task 6: signer — RFC 0019 `sighash_all`

**Files:**
- Create: `crates/signer-secp256k1/src/sighash.rs`
- Modify: `crates/signer-secp256k1/src/lib.rs`

**Interfaces:**
- Consumes: `recover`, `blake160` (Tasks 3, 4)
- Produces: `fn sighash_all(tx_hash: &[u8; 32], first_witness: &[u8], others: &[&[u8]]) -> [u8; 32]`

- [ ] **Step 1: Write the failing tests**

Create `crates/signer-secp256k1/src/sighash.rs`:

```rust
//! The `secp256k1_blake160_sighash_all` signing message (RFC 0019), exactly
//! as ckb-sdk-rust's `generate_message` computes it:
//!
//! `blake2b_256(tx_hash ‖ u64le(len(w0)) ‖ w0 ‖ Σ (u64le(len(wi)) ‖ wi))`
//!
//! `w0` is the first witness of the script group with its lock field
//! already replaced by `witness_lock_len` zero bytes by the caller.
//! `others` are the remaining witnesses of the group followed by every
//! witness beyond the input count. This module takes bytes; molecule
//! layout is the transaction builder's concern (plan 1e).

#[cfg(test)]
mod tests {
    use super::sighash_all;
    use crate::hash::blake160;
    use crate::sign::recover;

    // CKB testnet block 22356763, a one-input secp256k1_blake160 transfer.
    const TX_HASH: &str = "03e1abe59be2f5541d84590222048b4594318fa323e5ab0d377904cb84e624f4";
    const WITNESS: &str = "5500000010000000550000005500000041000000a0d661612ec85fc91e2569cd41d25d7704ed44c0042a79e78dc4dcb8e76fd1a15861db4e81cf318be154818199843007f00085d25e3a778494f9f770a4647a7801";
    const LOCK_ARGS: &str = "72f72b0cafd31de5072b10e84fc6c9d7d7596db7";

    fn h32(s: &str) -> [u8; 32] {
        let v = hex::decode(s).expect("hex");
        let mut out = [0u8; 32];
        out.copy_from_slice(&v);
        out
    }

    #[test]
    fn on_chain_signature_recovers_to_input_lock_args() {
        let witness = hex::decode(WITNESS).expect("hex");
        assert_eq!(witness.len(), 85);
        assert_eq!(
            &witness[..20],
            &hex::decode("5500000010000000550000005500000041000000").expect("hex")[..],
            "WitnessArgs header: total 0x55, offsets 0x10/0x55/0x55, lock len 0x41"
        );
        let mut signature = [0u8; 65];
        signature.copy_from_slice(&witness[20..85]);

        let mut zeroed = witness.clone();
        zeroed[20..85].fill(0);

        let digest = sighash_all(&h32(TX_HASH), &zeroed, &[]);
        let pk = recover(&signature, &digest).expect("real signature recovers");
        assert_eq!(hex::encode(blake160(&pk)), LOCK_ARGS);
    }

    #[test]
    fn other_witnesses_change_the_digest_and_are_order_sensitive() {
        let tx = [7u8; 32];
        let w0 = [1u8; 85];
        let a = [2u8; 10];
        let b = [3u8; 12];
        let none = sighash_all(&tx, &w0, &[]);
        let ab = sighash_all(&tx, &w0, &[&a, &b]);
        let ba = sighash_all(&tx, &w0, &[&b, &a]);
        assert_ne!(none, ab);
        assert_ne!(ab, ba);
    }
}
```

Add to `lib.rs`: `mod sighash;` and `pub use sighash::sighash_all;`.

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p lantern-signer-secp256k1`
Expected: compile error, `sighash_all` not found.

- [ ] **Step 3: Write the implementation (above the test module in `sighash.rs`)**

```rust
fn len_prefix(bytes: &[u8]) -> [u8; 8] {
    u64::try_from(bytes.len())
        .expect("witness length fits in u64")
        .to_le_bytes()
}

/// RFC 0019 signing message. See the module doc for the byte layout.
pub fn sighash_all(tx_hash: &[u8; 32], first_witness: &[u8], others: &[&[u8]]) -> [u8; 32] {
    let mut hasher = ckb_hash::new_blake2b();
    hasher.update(tx_hash);
    hasher.update(&len_prefix(first_witness));
    hasher.update(first_witness);
    for witness in others {
        hasher.update(&len_prefix(witness));
        hasher.update(witness);
    }
    let mut out = [0u8; 32];
    hasher.finalize(&mut out);
    out
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p lantern-signer-secp256k1`
Expected: 15 tests pass. The on-chain test is the crate's only external proof that sign, recover, blake160 and sighash agree with the network; if it fails, check the zeroing range first (`[20..85]`), then the length prefix (8 bytes, little-endian, applied to the first witness too).

- [ ] **Step 5: Clippy and commit**

Run: `cargo clippy -p lantern-signer-secp256k1 --all-targets -- -D warnings`
Expected: clean.

```bash
cargo fmt --all
git add crates/signer-secp256k1
git commit -m "feat(signer-secp256k1): RFC 0019 sighash_all verified against a testnet transaction"
```

---

### Task 7: signer — `Secp256k1Lock` implements `LockModule`

**Files:**
- Create: `crates/signer-secp256k1/src/lock.rs`
- Modify: `crates/signer-secp256k1/src/lib.rs`

**Interfaces:**
- Consumes: `LockModule`, `ScriptTemplate`, `SeedKind`, `LockError`, `Derivation` (Task 2); `derive_ckb_key`, `Branch`, `public_key`, `blake160`, `sign_recoverable` (Tasks 3 to 5)
- Produces:
  - `const SECP256K1_BLAKE160_CODE_HASH: [u8; 32]`, `const HASH_TYPE_TYPE: u8 = 0x01`
  - `struct Secp256k1Lock;` implementing `LockModule` with `extension_id = "core.secp256k1"`, `seed_kind = Bip39Seed`, `witness_lock_len = 65`

- [ ] **Step 1: Write the failing tests**

Create `crates/signer-secp256k1/src/lock.rs`:

```rust
//! `LockModule` implementation for `secp256k1_blake160_sighash_all`.

#[cfg(test)]
mod tests {
    use lantern_sdk_schema::{Derivation, LockError, LockModule, LockType, SeedKind};

    use super::{HASH_TYPE_TYPE, SECP256K1_BLAKE160_CODE_HASH, Secp256k1Lock};
    use crate::hash::blake160;
    use crate::key::{SigningKey, public_key};
    use crate::sign::recover;

    const TANK_SEED: &str = "1371018cfad5990f5e451bf586d59c3820a8671162d8700533549b0df61a63330e5cd5099a5d3938f833d51e4572104868bfac7cfe5b4063b1509a995652bc08";

    fn d(change: u32, index: u32) -> Derivation {
        Derivation { change, index }
    }

    #[test]
    fn identity() {
        let m = Secp256k1Lock;
        assert_eq!(m.lock_type(), LockType::Secp256k1Blake160);
        assert_eq!(m.extension_id(), "core.secp256k1");
        assert_eq!(m.seed_kind(), SeedKind::Bip39Seed);
        assert_eq!(m.witness_lock_len(), 65);
        assert!(m.capabilities().can_sign);
        assert!(!m.capabilities().hardware);
        let t = m.script_template();
        assert_eq!(t.code_hash, SECP256K1_BLAKE160_CODE_HASH);
        assert_eq!(t.hash_type, HASH_TYPE_TYPE);
        assert_eq!(
            hex::encode(t.code_hash),
            "9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8"
        );
    }

    #[test]
    fn lock_args_match_the_lumos_derived_key() {
        let seed = hex::decode(TANK_SEED).expect("hex");
        let args = Secp256k1Lock.derive_lock_args(&seed, &d(0, 0)).expect("derives");
        let key_bytes = hex::decode("848422863825f69e66dc7f48a3302459ec845395370c23578817456ad6b04b14")
            .expect("hex");
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&key_bytes);
        let expected = blake160(&public_key(&SigningKey::from_bytes(arr).expect("valid")));
        assert_eq!(args, expected.to_vec());
    }

    #[test]
    fn signature_recovers_to_lock_args() {
        let seed = hex::decode(TANK_SEED).expect("hex");
        let digest = [0x99u8; 32];
        let sig = Secp256k1Lock.sign_digest(&seed, &d(0, 3), &digest).expect("signs");
        assert_eq!(sig.len(), 65);
        let mut arr = [0u8; 65];
        arr.copy_from_slice(&sig);
        let pk = recover(&arr, &digest).expect("recovers");
        let args = Secp256k1Lock.derive_lock_args(&seed, &d(0, 3)).expect("derives");
        assert_eq!(blake160(&pk).to_vec(), args);
    }

    #[test]
    fn rejects_wrong_seed_shape_and_bad_branch() {
        assert_eq!(
            Secp256k1Lock.derive_lock_args(&[0u8; 32], &d(0, 0)).err(),
            Some(LockError::InvalidSeed)
        );
        let seed = hex::decode(TANK_SEED).expect("hex");
        assert_eq!(
            Secp256k1Lock.derive_lock_args(&seed, &d(2, 0)).err(),
            Some(LockError::InvalidDerivation)
        );
        assert_eq!(
            Secp256k1Lock.derive_lock_args(&seed, &d(0, 0x8000_0000)).err(),
            Some(LockError::InvalidDerivation)
        );
    }
}
```

Add to `lib.rs`: `mod lock;` and `pub use lock::{HASH_TYPE_TYPE, SECP256K1_BLAKE160_CODE_HASH, Secp256k1Lock};`.

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p lantern-signer-secp256k1`
Expected: compile error, `Secp256k1Lock` not found.

- [ ] **Step 3: Write the implementation (above the test module in `lock.rs`)**

```rust
use lantern_sdk_schema::{
    AccountCapabilities, Derivation, LockError, LockModule, LockType, ScriptTemplate, SeedKind,
};

use crate::error::SignerError;
use crate::hash::blake160;
use crate::hd::{Branch, derive_ckb_key};
use crate::key::{SigningKey, public_key};
use crate::sign::sign_recoverable;

/// Code hash of the system `secp256k1_blake160_sighash_all` script. The same
/// value on mainnet and testnet (it is a type-id hash, not a data hash).
pub const SECP256K1_BLAKE160_CODE_HASH: [u8; 32] = [
    0x9b, 0xd7, 0xe0, 0x6f, 0x3e, 0xcf, 0x4b, 0xe0, 0xf2, 0xfc, 0xd2, 0x18, 0x8b, 0x23, 0xf1, 0xb9,
    0xfc, 0xc8, 0x8e, 0x5d, 0x4b, 0x65, 0xa8, 0x63, 0x7b, 0x17, 0x72, 0x3b, 0xbd, 0xa3, 0xcc, 0xe8,
];

/// `ScriptHashType::Type`.
pub const HASH_TYPE_TYPE: u8 = 0x01;

const BIP39_SEED_LEN: usize = 64;
const WITNESS_LOCK_LEN: usize = 65;

/// The first-party secp256k1 lock module. Carries no state.
#[derive(Debug, Default, Clone, Copy)]
pub struct Secp256k1Lock;

fn key_for(seed: &[u8], derivation: &Derivation) -> Result<SigningKey, LockError> {
    if seed.len() != BIP39_SEED_LEN {
        return Err(LockError::InvalidSeed);
    }
    let branch = Branch::try_from(derivation.change).map_err(|_| LockError::InvalidDerivation)?;
    derive_ckb_key(seed, branch, derivation.index).map_err(|e| match e {
        SignerError::DerivationOverflow => LockError::InvalidDerivation,
        SignerError::InvalidSeedLength => LockError::InvalidSeed,
        other => LockError::Signing(other.to_string()),
    })
}

impl LockModule for Secp256k1Lock {
    fn lock_type(&self) -> LockType {
        LockType::Secp256k1Blake160
    }

    fn extension_id(&self) -> &'static str {
        "core.secp256k1"
    }

    fn capabilities(&self) -> AccountCapabilities {
        AccountCapabilities {
            can_sign: true,
            hardware: false,
        }
    }

    fn script_template(&self) -> ScriptTemplate {
        ScriptTemplate {
            code_hash: SECP256K1_BLAKE160_CODE_HASH,
            hash_type: HASH_TYPE_TYPE,
        }
    }

    fn seed_kind(&self) -> SeedKind {
        SeedKind::Bip39Seed
    }

    fn witness_lock_len(&self) -> usize {
        WITNESS_LOCK_LEN
    }

    fn derive_lock_args(&self, seed: &[u8], derivation: &Derivation) -> Result<Vec<u8>, LockError> {
        let key = key_for(seed, derivation)?;
        Ok(blake160(&public_key(&key)).to_vec())
    }

    fn sign_digest(
        &self,
        seed: &[u8],
        derivation: &Derivation,
        digest: &[u8; 32],
    ) -> Result<Vec<u8>, LockError> {
        let key = key_for(seed, derivation)?;
        Ok(sign_recoverable(&key, digest).to_vec())
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p lantern-signer-secp256k1`
Expected: 19 tests pass.

- [ ] **Step 5: Clippy and commit**

Run: `cargo clippy -p lantern-signer-secp256k1 --all-targets -- -D warnings`
Expected: clean.

```bash
cargo fmt --all
git add crates/signer-secp256k1
git commit -m "feat(signer-secp256k1): Secp256k1Lock implements LockModule"
```

---

### Task 8: registry — CKB2021 full-format address encoding

**Files:**
- Create: `crates/account-registry/src/error.rs`
- Create: `crates/account-registry/src/address.rs`
- Replace: `crates/account-registry/src/lib.rs`

**Interfaces:**
- Consumes: `Network`, `ScriptTemplate` (Task 2)
- Produces:
  - `RegistryError::{Io(std::io::Error), Corrupt, DuplicateId, NotFound, Address}`
  - `fn encode_full(network: Network, template: &ScriptTemplate, args: &[u8]) -> Result<String, RegistryError>`

- [ ] **Step 1: Write the failing tests**

Create `crates/account-registry/src/address.rs`:

```rust
//! CKB2021 full-format address: bech32m over
//! `0x00 ‖ code_hash ‖ hash_type ‖ args` with hrp `ckb` or `ckt` (RFC 0021).

#[cfg(test)]
mod tests {
    use lantern_sdk_schema::{Network, ScriptTemplate};

    use super::encode_full;

    fn secp_template() -> ScriptTemplate {
        let v = hex::decode("9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8")
            .expect("hex");
        let mut code_hash = [0u8; 32];
        code_hash.copy_from_slice(&v);
        ScriptTemplate {
            code_hash,
            hash_type: 0x01,
        }
    }

    #[test]
    fn rfc21_full_address_vector() {
        let args = hex::decode("b39bbc0b3673c7d36450bc14cfcdad2d559c6c64").expect("hex");
        assert_eq!(
            encode_full(Network::Mainnet, &secp_template(), &args).expect("encodes"),
            "ckb1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqdnnw7qkdnnclfkg59uzn8umtfd2kwxceqxwquc4"
        );
        assert_eq!(
            encode_full(Network::Testnet, &secp_template(), &args).expect("encodes"),
            "ckt1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqdnnw7qkdnnclfkg59uzn8umtfd2kwxceqgutnjd"
        );
    }

    #[test]
    fn lumos_first_receiving_address() {
        // blake160 of pub 0331b3c0… (lumos m/44'/309'/0'/0/0 from the BIP32 TV1 seed)
        let args = hex::decode("02e830bd6fe19912ffb7b0b134cbe53178b9e8f1").expect("hex");
        assert_eq!(
            encode_full(Network::Testnet, &secp_template(), &args).expect("encodes"),
            "ckt1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqgzaqct6mlpnyf0ldasky6vhef30zu73ugvy42t5"
        );
        assert_eq!(
            encode_full(Network::Mainnet, &secp_template(), &args).expect("encodes"),
            "ckb1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqgzaqct6mlpnyf0ldasky6vhef30zu73ugzk79pv"
        );
    }
}
```

Replace `crates/account-registry/src/lib.rs`:

```rust
#![forbid(unsafe_code)]

//! Lantern account registry.
//!
//! Plaintext, public-only storage of `StoredAccount` records beside the
//! vault, plus the projection to the IPC `AccountRecord`. Never holds a
//! secret and never names a concrete lock scheme: callers pass the
//! `ScriptTemplate` and capabilities they obtained from the account's
//! `LockModule`.

mod address;
mod error;

pub use address::encode_full;
pub use error::RegistryError;
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p lantern-account-registry`
Expected: compile error, `error` module and `encode_full` missing.

- [ ] **Step 3: Write `crates/account-registry/src/error.rs`**

```rust
//! Error type. `Display` never carries file contents.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("accounts file is corrupt or has an unsupported version")]
    Corrupt,

    #[error("an account with this id already exists")]
    DuplicateId,

    #[error("account not found")]
    NotFound,

    #[error("address encoding failed")]
    Address,
}
```

- [ ] **Step 4: Write the implementation (above the test module in `address.rs`)**

```rust
use bech32::{Bech32m, Hrp};
use lantern_sdk_schema::{Network, ScriptTemplate};

use crate::error::RegistryError;

const FULL_FORMAT_TYPE: u8 = 0x00;

/// Encode a lock script as a CKB2021 full-format address.
pub fn encode_full(
    network: Network,
    template: &ScriptTemplate,
    args: &[u8],
) -> Result<String, RegistryError> {
    let mut payload = Vec::with_capacity(34 + args.len());
    payload.push(FULL_FORMAT_TYPE);
    payload.extend_from_slice(&template.code_hash);
    payload.push(template.hash_type);
    payload.extend_from_slice(args);
    let hrp = Hrp::parse(network.address_prefix()).map_err(|_| RegistryError::Address)?;
    bech32::encode::<Bech32m>(hrp, &payload).map_err(|_| RegistryError::Address)
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p lantern-account-registry`
Expected: 2 tests pass.

- [ ] **Step 6: Clippy and commit**

Run: `cargo clippy -p lantern-account-registry --all-targets -- -D warnings`
Expected: clean.

```bash
cargo fmt --all
git add crates/account-registry
git commit -m "feat(account-registry): CKB2021 full address encoding with RFC 21 vectors"
```

---

### Task 9: registry — `StoredAccount`, `AccountRegistry`, `to_record`

**Files:**
- Create: `crates/account-registry/src/store.rs`
- Create: `crates/account-registry/src/record.rs`
- Modify: `crates/account-registry/src/lib.rs`

**Interfaces:**
- Consumes: `encode_full` (Task 8); `AccountRecord`, `AccountCapabilities`, `Derivation`, `LockType`, `Network`, `ScriptTemplate` (Task 2)
- Produces:
  - `StoredAccount { id: String, label: String, lock_type: LockType, extension_id: String, lock_args: Vec<u8>, derivation: Option<Derivation>, created_at: u64 }`
  - `fn account_id(lock_type: LockType, lock_args: &[u8]) -> String` → `"<slug>-<hex args>"`
  - `AccountRegistry`: `open(path) -> Result<Self>`, `add(StoredAccount) -> Result<()>`, `list(&self) -> &[StoredAccount]`, `get(&self, id) -> Option<&StoredAccount>`, `set_label(&mut self, id, label) -> Result<()>`, `remove(&mut self, id) -> Result<StoredAccount>`, `next_index(&self, change: u32) -> u32`, `save(&self) -> Result<()>`
  - `fn to_record(account: &StoredAccount, network: Network, template: &ScriptTemplate, capabilities: AccountCapabilities) -> Result<AccountRecord, RegistryError>` with `public_metadata = { "lockArgs": "0x…", "derivation": { "change": n, "index": n } }`

- [ ] **Step 1: Write the failing tests**

Create `crates/account-registry/src/store.rs`:

```rust
//! On-disk account list: `accounts.json` = `{ "version": 1, "accounts": [...] }`.
//! Public data only. Writes are tmp-then-rename.

#[cfg(test)]
mod tests {
    use lantern_sdk_schema::{Derivation, LockType};
    use tempfile::tempdir;

    use super::{AccountRegistry, StoredAccount, account_id};
    use crate::error::RegistryError;

    fn acct(index: u32, args: u8) -> StoredAccount {
        let lock_args = vec![args; 20];
        StoredAccount {
            id: account_id(LockType::Secp256k1Blake160, &lock_args),
            label: format!("Account {index}"),
            lock_type: LockType::Secp256k1Blake160,
            extension_id: "core.secp256k1".into(),
            lock_args,
            derivation: Some(Derivation { change: 0, index }),
            created_at: 1_700_000_000,
        }
    }

    #[test]
    fn id_is_slug_dash_hex() {
        assert_eq!(
            account_id(LockType::Secp256k1Blake160, &[0xab, 0xcd]),
            "secp256k1_blake160-abcd"
        );
    }

    #[test]
    fn missing_file_opens_empty() {
        let dir = tempdir().expect("tempdir");
        let reg = AccountRegistry::open(dir.path().join("accounts.json")).expect("opens");
        assert!(reg.list().is_empty());
        assert_eq!(reg.next_index(0), 0);
    }

    #[test]
    fn round_trips_through_disk_with_hex_args_and_camel_case() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("accounts.json");
        {
            let mut reg = AccountRegistry::open(&path).expect("opens");
            reg.add(acct(0, 0x11)).expect("adds");
            reg.add(acct(1, 0x22)).expect("adds");
            reg.save().expect("saves");
        }
        let text = std::fs::read_to_string(&path).expect("reads");
        assert!(text.contains("\"version\": 1"), "{text}");
        assert!(text.contains("\"lockArgs\": \"1111111111111111111111111111111111111111\""), "{text}");
        assert!(text.contains("\"lockType\": \"secp256k1_blake160\""), "{text}");
        assert!(!path.with_extension("json.tmp").exists(), "tmp file left behind");

        let reg = AccountRegistry::open(&path).expect("reopens");
        assert_eq!(reg.list(), &[acct(0, 0x11), acct(1, 0x22)]);
        assert_eq!(reg.get(&acct(1, 0x22).id), Some(&acct(1, 0x22)));
    }

    #[test]
    fn duplicate_id_is_rejected() {
        let dir = tempdir().expect("tempdir");
        let mut reg = AccountRegistry::open(dir.path().join("accounts.json")).expect("opens");
        reg.add(acct(0, 0x11)).expect("adds");
        assert!(matches!(reg.add(acct(5, 0x11)), Err(RegistryError::DuplicateId)));
    }

    #[test]
    fn corrupt_or_wrong_version_is_an_error_not_an_empty_registry() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("accounts.json");
        std::fs::write(&path, b"{ not json").expect("writes");
        assert!(matches!(AccountRegistry::open(&path), Err(RegistryError::Corrupt)));
        std::fs::write(&path, br#"{ "version": 2, "accounts": [] }"#).expect("writes");
        assert!(matches!(AccountRegistry::open(&path), Err(RegistryError::Corrupt)));
    }

    #[test]
    fn label_update_and_remove() {
        let dir = tempdir().expect("tempdir");
        let mut reg = AccountRegistry::open(dir.path().join("accounts.json")).expect("opens");
        reg.add(acct(0, 0x11)).expect("adds");
        let id = acct(0, 0x11).id;
        reg.set_label(&id, "Savings").expect("relabels");
        assert_eq!(reg.get(&id).map(|a| a.label.as_str()), Some("Savings"));
        assert!(matches!(reg.set_label("nope", "x"), Err(RegistryError::NotFound)));
        let removed = reg.remove(&id).expect("removes");
        assert_eq!(removed.label, "Savings");
        assert!(reg.list().is_empty());
        assert!(matches!(reg.remove(&id), Err(RegistryError::NotFound)));
    }

    #[test]
    fn next_index_skips_gaps_and_is_per_branch() {
        let dir = tempdir().expect("tempdir");
        let mut reg = AccountRegistry::open(dir.path().join("accounts.json")).expect("opens");
        reg.add(acct(0, 0x11)).expect("adds");
        reg.add(acct(1, 0x22)).expect("adds");
        reg.add(acct(5, 0x33)).expect("adds");
        assert_eq!(reg.next_index(0), 6);
        assert_eq!(reg.next_index(1), 0);
    }
}
```

Create `crates/account-registry/src/record.rs`:

```rust
//! Projection from the stored shape to the IPC `AccountRecord`. The address
//! is computed at read time so one stored record serves both networks.

#[cfg(test)]
mod tests {
    use lantern_sdk_schema::{AccountCapabilities, Derivation, LockType, Network, ScriptTemplate};

    use super::to_record;
    use crate::store::{StoredAccount, account_id};

    #[test]
    fn projects_rfc21_args_to_address_and_camel_case_metadata() {
        let v = hex::decode("9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8")
            .expect("hex");
        let mut code_hash = [0u8; 32];
        code_hash.copy_from_slice(&v);
        let template = ScriptTemplate {
            code_hash,
            hash_type: 0x01,
        };
        let lock_args = hex::decode("b39bbc0b3673c7d36450bc14cfcdad2d559c6c64").expect("hex");
        let stored = StoredAccount {
            id: account_id(LockType::Secp256k1Blake160, &lock_args),
            label: "Main".into(),
            lock_type: LockType::Secp256k1Blake160,
            extension_id: "core.secp256k1".into(),
            lock_args,
            derivation: Some(Derivation { change: 0, index: 4 }),
            created_at: 1,
        };
        let caps = AccountCapabilities {
            can_sign: true,
            hardware: false,
        };
        let record = to_record(&stored, Network::Mainnet, &template, caps).expect("projects");
        assert_eq!(
            record.address,
            "ckb1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqdnnw7qkdnnclfkg59uzn8umtfd2kwxceqxwquc4"
        );
        assert_eq!(record.id, stored.id);
        assert_eq!(record.capabilities, caps);
        assert_eq!(
            record.public_metadata,
            serde_json::json!({
                "lockArgs": "0xb39bbc0b3673c7d36450bc14cfcdad2d559c6c64",
                "derivation": { "change": 0, "index": 4 }
            })
        );
    }
}
```

Add to `lib.rs`: `mod record;`, `mod store;`, `pub use record::to_record;`, `pub use store::{AccountRegistry, StoredAccount, account_id};`. Add `hex.workspace = true` under `[dev-dependencies]` in `crates/account-registry/Cargo.toml` (it is already a normal dependency; tests use it too, so nothing else is needed).

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p lantern-account-registry`
Expected: compile error, `AccountRegistry` not found.

- [ ] **Step 3: Write the implementation (above the test module in `store.rs`)**

```rust
use std::fs;
use std::path::{Path, PathBuf};

use lantern_sdk_schema::{Derivation, LockType};
use serde::{Deserialize, Serialize};

use crate::error::RegistryError;

const FILE_VERSION: u32 = 1;

/// What `accounts.json` holds per account. Public data only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredAccount {
    pub id: String,
    pub label: String,
    pub lock_type: LockType,
    pub extension_id: String,
    #[serde(with = "hex::serde")]
    pub lock_args: Vec<u8>,
    pub derivation: Option<Derivation>,
    pub created_at: u64,
}

#[derive(Serialize, Deserialize)]
struct AccountsFile {
    version: u32,
    accounts: Vec<StoredAccount>,
}

/// Deterministic account id: `<lock slug>-<hex lock args>`.
pub fn account_id(lock_type: LockType, lock_args: &[u8]) -> String {
    format!("{}-{}", lock_type.slug(), hex::encode(lock_args))
}

/// In-memory account list bound to its file.
#[derive(Debug)]
pub struct AccountRegistry {
    path: PathBuf,
    accounts: Vec<StoredAccount>,
}

impl AccountRegistry {
    /// A missing file is an empty registry. A present but unreadable file
    /// is `Corrupt`; it is never silently replaced.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RegistryError> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Ok(Self {
                path,
                accounts: Vec::new(),
            });
        }
        let bytes = fs::read(&path)?;
        let file: AccountsFile =
            serde_json::from_slice(&bytes).map_err(|_| RegistryError::Corrupt)?;
        if file.version != FILE_VERSION {
            return Err(RegistryError::Corrupt);
        }
        Ok(Self {
            path,
            accounts: file.accounts,
        })
    }

    pub fn add(&mut self, account: StoredAccount) -> Result<(), RegistryError> {
        if self.get(&account.id).is_some() {
            return Err(RegistryError::DuplicateId);
        }
        self.accounts.push(account);
        Ok(())
    }

    pub fn list(&self) -> &[StoredAccount] {
        &self.accounts
    }

    pub fn get(&self, id: &str) -> Option<&StoredAccount> {
        self.accounts.iter().find(|a| a.id == id)
    }

    pub fn set_label(&mut self, id: &str, label: &str) -> Result<(), RegistryError> {
        let account = self
            .accounts
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or(RegistryError::NotFound)?;
        account.label = label.to_string();
        Ok(())
    }

    pub fn remove(&mut self, id: &str) -> Result<StoredAccount, RegistryError> {
        let position = self
            .accounts
            .iter()
            .position(|a| a.id == id)
            .ok_or(RegistryError::NotFound)?;
        Ok(self.accounts.remove(position))
    }

    /// One past the highest derived index on `change`, or 0.
    pub fn next_index(&self, change: u32) -> u32 {
        self.accounts
            .iter()
            .filter_map(|a| a.derivation)
            .filter(|d| d.change == change)
            .map(|d| d.index)
            .max()
            .map_or(0, |max| max + 1)
    }

    /// Write `accounts.json` via `accounts.json.tmp` + rename.
    pub fn save(&self) -> Result<(), RegistryError> {
        let file = AccountsFile {
            version: FILE_VERSION,
            accounts: self.accounts.clone(),
        };
        let json = serde_json::to_vec_pretty(&file).map_err(|_| RegistryError::Corrupt)?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        // Windows: `rename` fails when the destination exists. Plan 1f's
        // packaging work replaces this with a platform-aware atomic write.
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}
```

- [ ] **Step 4: Write the implementation (above the test module in `record.rs`)**

```rust
use lantern_sdk_schema::{AccountCapabilities, AccountRecord, Network, ScriptTemplate};
use serde_json::{Map, Value};

use crate::address::encode_full;
use crate::error::RegistryError;
use crate::store::StoredAccount;

/// Build the IPC projection of a stored account for `network`.
pub fn to_record(
    account: &StoredAccount,
    network: Network,
    template: &ScriptTemplate,
    capabilities: AccountCapabilities,
) -> Result<AccountRecord, RegistryError> {
    let address = encode_full(network, template, &account.lock_args)?;
    let mut metadata = Map::new();
    metadata.insert(
        "lockArgs".to_string(),
        Value::String(format!("0x{}", hex::encode(&account.lock_args))),
    );
    if let Some(derivation) = account.derivation {
        let value = serde_json::to_value(derivation).map_err(|_| RegistryError::Corrupt)?;
        metadata.insert("derivation".to_string(), value);
    }
    Ok(AccountRecord {
        id: account.id.clone(),
        label: account.label.clone(),
        lock_type: account.lock_type,
        extension_id: account.extension_id.clone(),
        address,
        public_metadata: Value::Object(metadata),
        capabilities,
    })
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p lantern-account-registry`
Expected: 10 tests pass.

- [ ] **Step 6: Clippy and commit**

Run: `cargo clippy -p lantern-account-registry --all-targets -- -D warnings`
Expected: clean.

```bash
cargo fmt --all
git add crates/account-registry
git commit -m "feat(account-registry): StoredAccount JSON store, deterministic ids, AccountRecord projection"
```

---

### Task 10: vault — `SecretSlice` from `get`, best-effort mlock, CI matrix

**Files:**
- Create: `crates/vault/src/mlock.rs`
- Modify: `crates/vault/src/vault.rs`
- Modify: `crates/vault/src/lib.rs`
- Modify: `crates/vault/tests/vault_roundtrip.rs`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Produces: `Vault::get(&self, name: &str) -> Option<SecretSlice<u8>>` (was `Option<Vec<u8>>`). Everything else on `Vault` is unchanged.

- [ ] **Step 1: Write the failing tests**

Create `crates/vault/src/mlock.rs`:

```rust
//! Best-effort page locking for long-lived unlocked state.
//!
//! With the `mlock` feature (default on) the vault asks the OS to keep the
//! pages holding the master key and every blob out of swap. A refusal (for
//! example Linux's default 64 KiB `RLIMIT_MEMLOCK`) is logged once and
//! ignored: a wallet that cannot open is worse than one that may page.
//!
//! Not covered: transient buffers (Argon2 output, decrypted CBOR). Those
//! are zeroized but never locked. A guard whose buffer was later
//! reallocated simply unlocks a stale range on drop; that is harmless.

use std::fmt;

#[cfg(feature = "mlock")]
pub struct PageLocks {
    guards: Vec<region::LockGuard>,
    warned: bool,
}

#[cfg(feature = "mlock")]
impl PageLocks {
    pub const fn new() -> Self {
        Self {
            guards: Vec::new(),
            warned: false,
        }
    }

    /// Lock the pages backing `bytes`. Empty slices are ignored.
    pub fn lock(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        match region::lock(bytes.as_ptr(), bytes.len()) {
            Ok(guard) => self.guards.push(guard),
            Err(err) => {
                if !self.warned {
                    self.warned = true;
                    tracing::warn!(
                        error = %err,
                        "mlock failed; unlocked vault state may be paged to disk"
                    );
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.guards.clear();
    }

    pub fn locked_regions(&self) -> usize {
        self.guards.len()
    }

    pub const fn warned(&self) -> bool {
        self.warned
    }
}

#[cfg(not(feature = "mlock"))]
pub struct PageLocks;

#[cfg(not(feature = "mlock"))]
#[allow(clippy::unused_self, clippy::missing_const_for_fn)]
impl PageLocks {
    pub const fn new() -> Self {
        Self
    }

    pub fn lock(&mut self, _bytes: &[u8]) {}

    pub fn clear(&mut self) {}

    pub const fn locked_regions(&self) -> usize {
        0
    }

    pub const fn warned(&self) -> bool {
        false
    }
}

impl fmt::Debug for PageLocks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PageLocks")
            .field("regions", &self.locked_regions())
            .field("warned", &self.warned())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::PageLocks;

    #[test]
    fn empty_slice_is_ignored() {
        let mut locks = PageLocks::new();
        locks.lock(&[]);
        assert_eq!(locks.locked_regions(), 0);
        assert!(!locks.warned());
    }

    #[cfg(feature = "mlock")]
    #[test]
    fn locks_then_clears_or_warns_exactly_once() {
        let mut locks = PageLocks::new();
        let a = vec![7u8; 64];
        let b = vec![9u8; 64];
        locks.lock(&a);
        locks.lock(&b);
        // Best-effort contract: every region locked, or a warning was raised.
        assert!(locks.locked_regions() == 2 || locks.warned());
        locks.clear();
        assert_eq!(locks.locked_regions(), 0);
    }
}
```

Add these tests to the `tests` module at the bottom of `crates/vault/src/vault.rs`:

```rust
    #[test]
    fn get_returns_secret_slice_with_opaque_debug() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault.bin");
        let mut v = Vault::create(&path, b"pw").unwrap();
        v.put("k", b"top secret");
        let got = v.get("k").expect("present");
        assert_eq!(got.expose_secret(), b"top secret");
        let dbg = format!("{got:?}");
        assert!(!dbg.contains("top secret"), "debug leaked the blob: {dbg}");
        assert!(v.get("missing").is_none());
    }

    #[test]
    fn page_locks_cover_master_key_and_every_blob_or_warn() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault.bin");
        let mut v = Vault::create(&path, b"pw").unwrap();
        v.put("a", b"1");
        v.put("b", b"22");
        let expected = if cfg!(feature = "mlock") { 3 } else { 0 };
        assert!(
            v.locks.locked_regions() == expected || v.locks.warned(),
            "regions={} warned={}",
            v.locks.locked_regions(),
            v.locks.warned()
        );
        v.lock();
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p lantern-vault`
Expected: compile errors: `mlock` module not declared, `v.locks` has no such field, `get` still returns `Vec<u8>` so `expose_secret` does not exist.

- [ ] **Step 3: Wire the module and update the crate doc in `crates/vault/src/lib.rs`**

Replace the module doc and module list so the file reads:

```rust
#![forbid(unsafe_code)]

//! Lantern vault.
//!
//! Encrypted at-rest storage for secret material. See plan 1b for the file
//! format and plan 1c for memory locking.
//!
//! Memory hygiene in force: zeroize on drop, `SecretBox`/`SecretSlice`
//! wrapping, no `Debug` leakage, and (with the default `mlock` feature)
//! best-effort page locking of the master key and every blob while the
//! vault is unlocked. Transient buffers are zeroized but not locked.

pub(crate) mod aead;
pub mod error;
pub(crate) mod format;
pub(crate) mod kdf;
pub(crate) mod mlock;
#[allow(dead_code)]
pub(crate) mod secret;
pub(crate) mod subkey;
pub mod vault;

pub use error::VaultError;
pub use secrecy::{ExposeSecret, SecretBox, SecretSlice};
pub use vault::Vault;
```

The `secrecy` re-export lets callers name the return type of `get` without adding the crate themselves.

- [ ] **Step 4: Update `crates/vault/src/vault.rs`**

Change the imports:

```rust
use secrecy::{ExposeSecret, ExposeSecretMut, SecretBox, SecretSlice};
use zeroize::{Zeroize, Zeroizing};

use crate::aead::{open, seal};
use crate::error::VaultError;
use crate::format::{HEADER_LEN, Header, NONCE_LEN, SALT_LEN};
use crate::kdf::derive_master_key;
use crate::mlock::PageLocks;
use crate::secret::InnerStore;
```

Change the module doc bullet that reads `//! - `mlock` is NOT performed in this plan (1b). See the module doc for` / `//!   why, and plan 1c for the follow-up.` to:

```rust
//! - With the `mlock` feature the master key and every blob are page-locked
//!   while unlocked (best effort; see `mlock.rs`). Guards are released after
//!   the secrets are zeroized because `locks` is the last field.
```

Add the field as the **last** field of the struct (drop order matters: secrets zeroize first, then the pages unlock):

```rust
#[derive(Debug)]
pub struct Vault {
    path: PathBuf,
    header: Header,
    master_key: SecretBox<[u8; 32]>,
    store: SecretBox<InnerStore>,
    locks: PageLocks,
}
```

In `create`, replace the block that builds `v` and saves:

```rust
        let store = SecretBox::new(Box::new(InnerStore::new()));
        let mut v = Self {
            path,
            header,
            master_key,
            store,
            locks: PageLocks::new(),
        };
        v.relock();
        v.save()?;
        Ok(v)
```

In `unlock`, replace the final `Ok(Self { ... })`:

```rust
        let mut v = Self {
            path,
            header,
            master_key,
            store: SecretBox::new(Box::new(store)),
            locks: PageLocks::new(),
        };
        v.relock();
        Ok(v)
```

Replace `put` and `get`:

```rust
    pub fn put(&mut self, name: &str, value: &[u8]) {
        self.store
            .expose_secret_mut()
            .blobs
            .insert(name.to_string(), value.to_vec());
        self.relock();
    }

    /// Copy of a blob inside a zeroizing, `Debug`-opaque wrapper.
    pub fn get(&self, name: &str) -> Option<SecretSlice<u8>> {
        self.store
            .expose_secret()
            .blobs
            .get(name)
            .map(|v| SecretSlice::from(v.clone()))
    }

    /// Re-lock the pages behind the master key and every blob. Called after
    /// any change to the store because `Vec` reallocation moves the bytes.
    fn relock(&mut self) {
        self.locks.clear();
        self.locks.lock(&self.master_key.expose_secret()[..]);
        for blob in self.store.expose_secret().blobs.values() {
            self.locks.lock(blob);
        }
    }
```

In `save`, add the Windows note above `fs::rename`:

```rust
        // Windows: `rename` fails when the destination exists. Plan 1f's
        // packaging work replaces this with a platform-aware atomic write.
        fs::rename(&tmp, &self.path)?;
```

Update the two existing unit tests that call `get`:

```rust
        assert!(v.get("anything").is_none());
```

and

```rust
        assert_eq!(
            v.get("mnemonic").map(|s| s.expose_secret().to_vec()),
            Some(b"abandon abandon abandon".to_vec())
        );
```

- [ ] **Step 5: Update `crates/vault/tests/vault_roundtrip.rs`**

Add at the top of the file:

```rust
use lantern_vault::ExposeSecret;
```

and a helper below the imports:

```rust
fn blob(v: &lantern_vault::Vault, name: &str) -> Option<Vec<u8>> {
    v.get(name).map(|s| s.expose_secret().to_vec())
}
```

Then replace every `v.get(NAME).as_deref()` with `blob(&v, NAME).as_deref()` and `v.get("not-there")` with `blob(&v, "not-there")`. There are four call sites (lines 29, 32, 33 and 99 at the time of writing). The assertions keep their expected values.

- [ ] **Step 6: Run the tests in both feature configurations**

Run: `cargo test -p lantern-vault`
Expected: 38 tests pass (34 existing + 2 in `vault.rs` + 2 in `mlock.rs`).

Run: `cargo test -p lantern-vault --no-default-features`
Expected: 37 tests pass (the feature-gated `PageLocks` test is skipped).

- [ ] **Step 7: Add the CI matrix entry**

In `.github/workflows/ci.yml`, after the `cargo test` step of the `rust` job, add:

```yaml
      - name: cargo clippy (vault without mlock)
        run: cargo clippy -p lantern-vault --all-targets --no-default-features -- -D warnings

      - name: cargo test (vault without mlock)
        run: cargo test -p lantern-vault --no-default-features
```

- [ ] **Step 8: Clippy in both configurations and commit**

Run: `cargo clippy -p lantern-vault --all-targets -- -D warnings`
Run: `cargo clippy -p lantern-vault --all-targets --no-default-features -- -D warnings`
Expected: both clean.

```bash
cargo fmt --all
git add crates/vault .github/workflows/ci.yml
git commit -m "feat(vault): SecretSlice from get, best-effort mlock behind default feature"
```

---

### Task 11: wallet-core — mnemonic parse, render, and BIP39 seed

**Files:**
- Create: `crates/wallet-core/src/error.rs`
- Create: `crates/wallet-core/src/mnemonic.rs`
- Replace: `crates/wallet-core/src/lib.rs`

**Interfaces:**
- Consumes: `VaultError` (vault), `RegistryError` (Task 8), `LockError`, `LockType` (Task 2)
- Produces:
  - `CoreError::{Vault, Registry, Lock, AlreadyInitialised, SeedMissing, InvalidMnemonic, AccountNotFound, UnsupportedLock(LockType), NoSigningMaterial}`
  - `WordCount { Words12, Words24 }` with `const fn entropy_len(self) -> usize`
  - `MnemonicFormat { Single, Combined3 }`
  - `Phrase` (wraps `SecretString`; no Debug/Clone): `fn expose(&self) -> &str`, `fn word_count(&self) -> usize`
  - `const fn format_for_entropy(len: usize) -> Result<MnemonicFormat, CoreError>`
  - `fn generate_entropy(words: WordCount) -> Zeroizing<Vec<u8>>`
  - `fn parse_phrase(phrase: &str) -> Result<Zeroizing<Vec<u8>>, CoreError>`
  - `fn render_phrase(entropy: &[u8]) -> Result<Phrase, CoreError>`
  - `fn bip39_seed(entropy: &[u8]) -> Result<SecretBox<[u8; 64]>, CoreError>`

Memory note for the executor: `SecretString::from(String)` calls `into_boxed_str`, which may reallocate and leave the old buffer un-zeroized. Always build phrase text inside a `Zeroizing<String>` and convert with `SecretString::from(text.as_str())`, which allocates exactly once at the final length.

- [ ] **Step 1: Write the failing tests**

Create `crates/wallet-core/src/mnemonic.rs`:

```rust
//! Mnemonic handling for two formats:
//!
//! - **Single**: standard BIP39, 12/15/18/21/24 words, 16..=32 bytes of entropy.
//! - **Combined3**: Quantum Purse's format, 36/54/72 words = three equal
//!   standard phrases whose entropy blocks are concatenated (48/72/96 bytes).
//!   Byte-for-byte what `quantumpurse/key-vault-wasm` `import_seed_phrase`
//!   and `export_seed_phrase` do.
//!
//! The stored entropy length alone discriminates the formats (single tops
//! out at 32, combined starts at 48), so nothing else is persisted.
//!
//! The BIP39 seed is `PBKDF2-HMAC-SHA512(NFKD(phrase), "mnemonic", 2048, 64)`
//! over the **whole** phrase text. For standard lengths that is BIP39
//! exactly. For combined phrases it is Lantern's defined generalisation
//! (no other wallet derives a secp seed from those). The phrase is always
//! re-rendered from entropy, so it is canonical ASCII and NFKD is identity.

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;

    use super::{
        MnemonicFormat, WordCount, bip39_seed, format_for_entropy, generate_entropy, parse_phrase,
        render_phrase,
    };
    use crate::error::CoreError;

    const P1: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    const P2: &str = "legal winner thank year wave sausage worth useful legal winner thank yellow";
    const P3: &str = "letter advice cage absurd amount doctor acoustic avoid letter advice cage above";
    const P4: &str = "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo vote";
    const TANK: &str = "tank planet champion pottery together intact quick police asset flower sudden question";

    fn trezor_vectors() -> [(Vec<u8>, &'static str, &'static str); 4] {
        [
            (vec![0x00; 16], P1, "5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc19a5ac40b389cd370d086206dec8aa6c43daea6690f20ad3d8d48b2d2ce9e38e4"),
            (vec![0x7f; 16], P2, "878386efb78845b3355bd15ea4d39ef97d179cb712b77d5c12b6be415fffeffe5f377ba02bf3f8544ab800b955e51fbff09828f682052a20faa6addbbddfb096"),
            (vec![0x80; 16], P3, "77d6be9708c8218738934f84bbbb78a2e048ca007746cb764f0673e4b1812d176bbb173e1a291f31cf633f1d0bad7d3cf071c30e98cd0688b5bcce65ecaceb36"),
            (vec![0xff; 32], P4, "e28a37058c7f5112ec9e16a3437cf363a2572d70b6ceb3b6965447623d620f14d06bb321a26b33ec15fcd84a3b5ddfd5520e230c924c87aaa0d559749e044fef"),
        ]
    }

    #[test]
    fn trezor_vectors_round_trip_and_seed() {
        for (entropy, phrase, seed) in trezor_vectors() {
            let parsed = parse_phrase(phrase).expect("parses");
            assert_eq!(&*parsed, &entropy, "entropy for {phrase}");
            let rendered = render_phrase(&entropy).expect("renders");
            assert_eq!(rendered.expose(), phrase);
            let got = bip39_seed(&entropy).expect("seed");
            assert_eq!(hex::encode(got.expose_secret()), seed, "seed for {phrase}");
        }
    }

    #[test]
    fn lumos_tank_planet_seed() {
        let entropy = parse_phrase(TANK).expect("parses");
        let seed = bip39_seed(&entropy).expect("seed");
        assert_eq!(
            hex::encode(seed.expose_secret()),
            "1371018cfad5990f5e451bf586d59c3820a8671162d8700533549b0df61a63330e5cd5099a5d3938f833d51e4572104868bfac7cfe5b4063b1509a995652bc08"
        );
    }

    #[test]
    fn combined_36_words_concatenates_entropy_in_order() {
        let combined = format!("{P1} {P2} {P3}");
        let entropy = parse_phrase(&combined).expect("parses");
        let mut expected = vec![0x00u8; 16];
        expected.extend_from_slice(&[0x7f; 16]);
        expected.extend_from_slice(&[0x80; 16]);
        assert_eq!(&*entropy, &expected);
        assert_eq!(format_for_entropy(entropy.len()).expect("format"), MnemonicFormat::Combined3);
        let rendered = render_phrase(&entropy).expect("renders");
        assert_eq!(rendered.expose(), combined);
        assert_eq!(rendered.word_count(), 36);
        // Lantern-defined seed over the whole phrase; pinned with Python hashlib.
        let seed = bip39_seed(&entropy).expect("seed");
        assert_eq!(
            hex::encode(seed.expose_secret()),
            "4b4dbd0a319c456707c46f77d6c547267bbb667ecfebd5bf08941ff57ed592e63f7e42b59a38f9b9dd43f9986dd7c776c9805a7cff7a468e20e96a5018661354"
        );
    }

    #[test]
    fn combined_72_words_round_trips() {
        let entropy = vec![0xffu8; 96];
        let phrase = render_phrase(&entropy).expect("renders");
        assert_eq!(phrase.word_count(), 72);
        assert_eq!(phrase.expose(), format!("{P4} {P4} {P4}"));
        assert_eq!(&*parse_phrase(phrase.expose()).expect("parses"), &entropy);
    }

    #[test]
    fn rejects_unsupported_lengths_and_bad_checksums() {
        let thirteen = format!("{P1} abandon");
        assert!(matches!(parse_phrase(&thirteen), Err(CoreError::InvalidMnemonic)));
        let thirty = format!("{P1} {P2} legal winner thank year wave sausage");
        assert!(matches!(parse_phrase(&thirty), Err(CoreError::InvalidMnemonic)));
        let forty_eight = format!("{P1} {P1} {P1} {P1}");
        assert!(matches!(parse_phrase(&forty_eight), Err(CoreError::InvalidMnemonic)));
        let bad_checksum = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon";
        assert!(matches!(parse_phrase(bad_checksum), Err(CoreError::InvalidMnemonic)));
        let unknown_word = P1.replace("about", "lantern");
        assert!(matches!(parse_phrase(&unknown_word), Err(CoreError::InvalidMnemonic)));
        // A combined phrase whose second chunk has a bad checksum fails as a whole.
        let bad_middle = format!("{P1} {bad_checksum} {P3}");
        assert!(matches!(parse_phrase(&bad_middle), Err(CoreError::InvalidMnemonic)));
    }

    #[test]
    fn entropy_length_discriminates_format() {
        assert_eq!(format_for_entropy(16).expect("ok"), MnemonicFormat::Single);
        assert_eq!(format_for_entropy(20).expect("ok"), MnemonicFormat::Single);
        assert_eq!(format_for_entropy(32).expect("ok"), MnemonicFormat::Single);
        assert_eq!(format_for_entropy(48).expect("ok"), MnemonicFormat::Combined3);
        assert_eq!(format_for_entropy(72).expect("ok"), MnemonicFormat::Combined3);
        assert_eq!(format_for_entropy(96).expect("ok"), MnemonicFormat::Combined3);
        assert!(matches!(format_for_entropy(40), Err(CoreError::InvalidMnemonic)));
        assert!(matches!(render_phrase(&[0u8; 40]), Err(CoreError::InvalidMnemonic)));
        assert_eq!(render_phrase(&[0u8; 20]).expect("renders").word_count(), 15);
    }

    #[test]
    fn generated_entropy_has_the_requested_size_and_is_random() {
        assert_eq!(WordCount::Words12.entropy_len(), 16);
        assert_eq!(WordCount::Words24.entropy_len(), 32);
        let a = generate_entropy(WordCount::Words24);
        let b = generate_entropy(WordCount::Words24);
        assert_eq!(a.len(), 32);
        assert_ne!(&*a, &*b);
        assert_eq!(render_phrase(&a).expect("renders").word_count(), 24);
    }

    #[test]
    fn phrase_debug_is_opaque() {
        let phrase = render_phrase(&[0u8; 16]).expect("renders");
        let dbg = format!("{:?}", phrase.0);
        assert!(!dbg.contains("abandon"), "leaked: {dbg}");
    }
}
```

Create `crates/wallet-core/src/error.rs`:

```rust
//! Error type. Wraps the lower crates' errors and adds orchestration
//! failures. `Display` never carries phrase words or key bytes;
//! `InvalidMnemonic` deliberately drops the `bip39` detail, which can echo
//! the offending word.

use lantern_account_registry::RegistryError;
use lantern_sdk_schema::{LockError, LockType};
use lantern_vault::VaultError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error(transparent)]
    Vault(#[from] VaultError),

    #[error(transparent)]
    Registry(#[from] RegistryError),

    #[error(transparent)]
    Lock(#[from] LockError),

    #[error("wallet is already initialised")]
    AlreadyInitialised,

    #[error("no seed material in the vault")]
    SeedMissing,

    #[error("invalid mnemonic: expected 12, 15, 18, 21, 24, 36, 54 or 72 valid words")]
    InvalidMnemonic,

    #[error("account not found")]
    AccountNotFound,

    #[error("no lock module registered for {0:?}")]
    UnsupportedLock(LockType),

    #[error("account carries no signing material")]
    NoSigningMaterial,
}
```

Replace `crates/wallet-core/src/lib.rs`:

```rust
#![forbid(unsafe_code)]

//! Lantern wallet core.
//!
//! Orchestrates the vault, the account registry, and the lock modules.
//! Owns the seed lifecycle (`Keyring`), the module map (`LockRegistry`),
//! and the single signing entry point (`SigningCoordinator`).

pub mod error;
pub mod mnemonic;

pub use error::CoreError;
pub use mnemonic::{MnemonicFormat, Phrase, WordCount};
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p lantern-wallet-core`
Expected: compile error, `parse_phrase` and friends not found.

- [ ] **Step 3: Write the implementation (above the test module in `mnemonic.rs`)**

```rust
use bip39::{Language, Mnemonic};
use rand::{RngCore, rngs::OsRng};
use secrecy::{ExposeSecret, SecretBox, SecretString};
use sha2::Sha512;
use zeroize::{Zeroize, Zeroizing};

use crate::error::CoreError;

const PBKDF2_ROUNDS: u32 = 2048;
const PBKDF2_SALT: &[u8] = b"mnemonic";

/// Word counts Lantern generates. Import accepts more (see `parse_phrase`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordCount {
    Words12,
    Words24,
}

impl WordCount {
    pub const fn entropy_len(self) -> usize {
        match self {
            Self::Words12 => 16,
            Self::Words24 => 32,
        }
    }
}

/// Layout of a phrase, derived from the entropy length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MnemonicFormat {
    Single,
    Combined3,
}

/// Full phrase text. Zeroized on drop; `Debug` is opaque.
pub struct Phrase(SecretString);

impl Phrase {
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }

    pub fn word_count(&self) -> usize {
        self.expose().split_whitespace().count()
    }
}

/// Which format a stored entropy length belongs to.
pub const fn format_for_entropy(len: usize) -> Result<MnemonicFormat, CoreError> {
    match len {
        16 | 20 | 24 | 28 | 32 => Ok(MnemonicFormat::Single),
        48 | 72 | 96 => Ok(MnemonicFormat::Combined3),
        _ => Err(CoreError::InvalidMnemonic),
    }
}

/// Fresh OS entropy for a new wallet.
pub fn generate_entropy(words: WordCount) -> Zeroizing<Vec<u8>> {
    let mut entropy = Zeroizing::new(vec![0u8; words.entropy_len()]);
    OsRng.fill_bytes(&mut entropy);
    entropy
}

/// Parse a standard or combined phrase into its entropy.
pub fn parse_phrase(phrase: &str) -> Result<Zeroizing<Vec<u8>>, CoreError> {
    let words: Vec<&str> = phrase.split_whitespace().collect();
    let chunks = match words.len() {
        12 | 15 | 18 | 21 | 24 => 1,
        36 | 54 | 72 => 3,
        _ => return Err(CoreError::InvalidMnemonic),
    };
    let per_chunk = words.len() / chunks;
    let mut entropy = Zeroizing::new(Vec::with_capacity(per_chunk / 3 * 4 * chunks));
    for chunk in words.chunks(per_chunk) {
        let text = Zeroizing::new(chunk.join(" "));
        let mnemonic = Mnemonic::parse_in(Language::English, text.as_str())
            .map_err(|_| CoreError::InvalidMnemonic)?;
        let chunk_entropy = Zeroizing::new(mnemonic.to_entropy());
        entropy.extend_from_slice(&chunk_entropy);
    }
    Ok(entropy)
}

/// Render entropy as its canonical phrase (one or three chunks).
pub fn render_phrase(entropy: &[u8]) -> Result<Phrase, CoreError> {
    let chunk_len = match format_for_entropy(entropy.len())? {
        MnemonicFormat::Single => entropy.len(),
        MnemonicFormat::Combined3 => entropy.len() / 3,
    };
    let mut text = Zeroizing::new(String::new());
    for chunk in entropy.chunks(chunk_len) {
        let mnemonic = Mnemonic::from_entropy_in(Language::English, chunk)
            .map_err(|_| CoreError::InvalidMnemonic)?;
        for word in mnemonic.words() {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(word);
        }
    }
    // `From<&str>` allocates exactly once at the final length; `text`
    // zeroizes on drop. See the module note on `From<String>`.
    Ok(Phrase(SecretString::from(text.as_str())))
}

/// The 64-byte BIP39 seed for `entropy` (empty passphrase).
pub fn bip39_seed(entropy: &[u8]) -> Result<SecretBox<[u8; 64]>, CoreError> {
    let phrase = render_phrase(entropy)?;
    let mut seed = [0u8; 64];
    pbkdf2::pbkdf2_hmac::<Sha512>(phrase.expose().as_bytes(), PBKDF2_SALT, PBKDF2_ROUNDS, &mut seed);
    let boxed = SecretBox::new(Box::new(seed));
    seed.zeroize();
    Ok(boxed)
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p lantern-wallet-core`
Expected: 8 tests pass. If `trezor_vectors_round_trip_and_seed` fails on the seed only, check the salt is exactly `b"mnemonic"` and the round count is 2048; if it fails on the phrase, `Language::English` must be passed explicitly.

- [ ] **Step 5: Clippy and commit**

Run: `cargo clippy -p lantern-wallet-core --all-targets -- -D warnings`
Expected: clean. If clippy asks for `const fn` on `Phrase::expose`, it cannot be const (`expose_secret` is not const); add `#[allow(clippy::missing_const_for_fn)]` on that method only.

```bash
cargo fmt --all
git add crates/wallet-core
git commit -m "feat(wallet-core): BIP39 + Quantum Purse combined mnemonic parse/render/seed"
```

---

### Task 12: wallet-core — `Keyring` over the vault and `LockRegistry`

**Files:**
- Create: `crates/wallet-core/src/keyring.rs`
- Create: `crates/wallet-core/src/locks.rs`
- Modify: `crates/wallet-core/src/lib.rs`

**Interfaces:**
- Consumes: `Vault::{put, get, save}` (Task 10), Task 11 functions, `Secp256k1Lock` (Task 7), `LockModule`, `SeedKind`, `LockType` (Task 2)
- Produces:
  - `const ENTROPY_BLOB: &str = "keyring/entropy"`
  - `Keyring::is_initialised(&Vault) -> bool`, `create(&mut Vault, WordCount) -> Result<Phrase>`, `import(&mut Vault, &str) -> Result<MnemonicFormat>`, `entropy(&Vault) -> Result<SecretSlice<u8>>`, `format(&Vault) -> Result<MnemonicFormat>`, `phrase(&Vault) -> Result<Phrase>`, `bip39_seed(&Vault) -> Result<SecretBox<[u8; 64]>>`, `seed_for(&Vault, SeedKind) -> Result<SecretSlice<u8>>`
  - `LockRegistry::new()` (empty), `with_first_party()`, `register(Box<dyn LockModule>)`, `get(LockType) -> Result<&dyn LockModule, CoreError>`

- [ ] **Step 1: Write the failing tests**

Create `crates/wallet-core/src/keyring.rs`:

```rust
//! Seed lifecycle over the vault. One blob, `keyring/entropy`, holds the
//! BIP39 entropy (16..=32 bytes single, 48/72/96 combined). Everything
//! else (phrase, BIP39 seed) is regenerated on demand.

#[cfg(test)]
mod tests {
    use lantern_sdk_schema::SeedKind;
    use lantern_vault::{ExposeSecret, Vault};
    use tempfile::tempdir;

    use super::{ENTROPY_BLOB, Keyring};
    use crate::error::CoreError;
    use crate::mnemonic::{MnemonicFormat, WordCount};

    const TANK: &str = "tank planet champion pottery together intact quick police asset flower sudden question";

    #[test]
    fn create_stores_entropy_and_persists() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("vault.bin");
        let phrase_text = {
            let mut vault = Vault::create(&path, b"pw").expect("creates");
            assert!(!Keyring::is_initialised(&vault));
            let phrase = Keyring::create(&mut vault, WordCount::Words24).expect("creates");
            assert_eq!(phrase.word_count(), 24);
            assert!(Keyring::is_initialised(&vault));
            assert!(matches!(
                Keyring::create(&mut vault, WordCount::Words12),
                Err(CoreError::AlreadyInitialised)
            ));
            phrase.expose().to_string()
        };
        let vault = Vault::unlock(&path, b"pw").expect("unlocks");
        assert_eq!(Keyring::entropy(&vault).expect("entropy").expose_secret().len(), 32);
        assert_eq!(Keyring::format(&vault).expect("format"), MnemonicFormat::Single);
        assert_eq!(Keyring::phrase(&vault).expect("phrase").expose(), phrase_text);
    }

    #[test]
    fn import_known_phrase_yields_known_seed() {
        let dir = tempdir().expect("tempdir");
        let mut vault = Vault::create(dir.path().join("vault.bin"), b"pw").expect("creates");
        assert_eq!(Keyring::import(&mut vault, TANK).expect("imports"), MnemonicFormat::Single);
        let seed = Keyring::bip39_seed(&vault).expect("seed");
        assert_eq!(
            hex::encode(seed.expose_secret()),
            "1371018cfad5990f5e451bf586d59c3820a8671162d8700533549b0df61a63330e5cd5099a5d3938f833d51e4572104868bfac7cfe5b4063b1509a995652bc08"
        );
        assert!(matches!(Keyring::import(&mut vault, TANK), Err(CoreError::AlreadyInitialised)));
    }

    #[test]
    fn import_rejects_bad_phrase_without_touching_the_vault() {
        let dir = tempdir().expect("tempdir");
        let mut vault = Vault::create(dir.path().join("vault.bin"), b"pw").expect("creates");
        assert!(matches!(
            Keyring::import(&mut vault, "not a phrase"),
            Err(CoreError::InvalidMnemonic)
        ));
        assert!(!Keyring::is_initialised(&vault));
        assert!(vault.get(ENTROPY_BLOB).is_none());
    }

    #[test]
    fn seed_for_hands_each_kind_its_material() {
        let dir = tempdir().expect("tempdir");
        let mut vault = Vault::create(dir.path().join("vault.bin"), b"pw").expect("creates");
        Keyring::import(&mut vault, TANK).expect("imports");
        let raw = Keyring::seed_for(&vault, SeedKind::RawEntropy).expect("raw");
        assert_eq!(raw.expose_secret().len(), 16);
        let bip39 = Keyring::seed_for(&vault, SeedKind::Bip39Seed).expect("bip39");
        assert_eq!(bip39.expose_secret().len(), 64);
        assert_eq!(
            hex::encode(bip39.expose_secret()),
            "1371018cfad5990f5e451bf586d59c3820a8671162d8700533549b0df61a63330e5cd5099a5d3938f833d51e4572104868bfac7cfe5b4063b1509a995652bc08"
        );
    }

    #[test]
    fn empty_vault_reports_seed_missing() {
        let dir = tempdir().expect("tempdir");
        let vault = Vault::create(dir.path().join("vault.bin"), b"pw").expect("creates");
        assert!(matches!(Keyring::entropy(&vault), Err(CoreError::SeedMissing)));
        assert!(matches!(Keyring::phrase(&vault), Err(CoreError::SeedMissing)));
    }
}
```

Create `crates/wallet-core/src/locks.rs`:

```rust
//! Map from `LockType` to the module that serves it. First-party modules
//! are registered by `with_first_party`; tests and future extension hosts
//! register their own.

#[cfg(test)]
mod tests {
    use lantern_sdk_schema::{LockModule, LockType};

    use super::LockRegistry;
    use crate::error::CoreError;

    #[test]
    fn first_party_registry_serves_secp256k1() {
        let registry = LockRegistry::with_first_party();
        let module = registry.get(LockType::Secp256k1Blake160).expect("registered");
        assert_eq!(module.extension_id(), "core.secp256k1");
        assert_eq!(module.witness_lock_len(), 65);
    }

    #[test]
    fn empty_registry_reports_unsupported_lock() {
        let registry = LockRegistry::new();
        assert!(matches!(
            registry.get(LockType::Secp256k1Blake160),
            Err(CoreError::UnsupportedLock(LockType::Secp256k1Blake160))
        ));
    }
}
```

Add to `lib.rs`: `pub mod keyring;`, `pub mod locks;`, `pub use keyring::Keyring;`, `pub use locks::LockRegistry;`.

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p lantern-wallet-core`
Expected: compile error, `Keyring` and `LockRegistry` not found.

- [ ] **Step 3: Write the implementation (above the test module in `keyring.rs`)**

```rust
use lantern_sdk_schema::SeedKind;
use lantern_vault::{ExposeSecret, SecretBox, SecretSlice, Vault};

use crate::error::CoreError;
use crate::mnemonic::{
    self, MnemonicFormat, Phrase, WordCount, format_for_entropy, generate_entropy, parse_phrase,
    render_phrase,
};

/// Vault blob name for the mnemonic entropy.
pub const ENTROPY_BLOB: &str = "keyring/entropy";

/// Stateless namespace for seed operations on an unlocked vault.
pub struct Keyring;

impl Keyring {
    pub fn is_initialised(vault: &Vault) -> bool {
        vault.get(ENTROPY_BLOB).is_some()
    }

    /// Generate a fresh wallet. Returns the phrase exactly once.
    pub fn create(vault: &mut Vault, words: WordCount) -> Result<Phrase, CoreError> {
        if Self::is_initialised(vault) {
            return Err(CoreError::AlreadyInitialised);
        }
        let entropy = generate_entropy(words);
        vault.put(ENTROPY_BLOB, &entropy);
        vault.save()?;
        render_phrase(&entropy)
    }

    /// Import a standard or combined phrase.
    pub fn import(vault: &mut Vault, phrase: &str) -> Result<MnemonicFormat, CoreError> {
        if Self::is_initialised(vault) {
            return Err(CoreError::AlreadyInitialised);
        }
        let entropy = parse_phrase(phrase)?;
        let format = format_for_entropy(entropy.len())?;
        vault.put(ENTROPY_BLOB, &entropy);
        vault.save()?;
        Ok(format)
    }

    pub fn entropy(vault: &Vault) -> Result<SecretSlice<u8>, CoreError> {
        vault.get(ENTROPY_BLOB).ok_or(CoreError::SeedMissing)
    }

    pub fn format(vault: &Vault) -> Result<MnemonicFormat, CoreError> {
        let entropy = Self::entropy(vault)?;
        format_for_entropy(entropy.expose_secret().len())
    }

    pub fn phrase(vault: &Vault) -> Result<Phrase, CoreError> {
        let entropy = Self::entropy(vault)?;
        render_phrase(entropy.expose_secret())
    }

    pub fn bip39_seed(vault: &Vault) -> Result<SecretBox<[u8; 64]>, CoreError> {
        let entropy = Self::entropy(vault)?;
        mnemonic::bip39_seed(entropy.expose_secret())
    }

    /// Exactly the material a `LockModule` asked for, nothing else.
    pub fn seed_for(vault: &Vault, kind: SeedKind) -> Result<SecretSlice<u8>, CoreError> {
        match kind {
            SeedKind::RawEntropy => Self::entropy(vault),
            SeedKind::Bip39Seed => {
                let seed = Self::bip39_seed(vault)?;
                Ok(SecretSlice::from(seed.expose_secret().to_vec()))
            }
        }
    }
}
```

- [ ] **Step 4: Write the implementation (above the test module in `locks.rs`)**

```rust
use std::collections::BTreeMap;

use lantern_sdk_schema::{LockModule, LockType};
use lantern_signer_secp256k1::Secp256k1Lock;

use crate::error::CoreError;

/// Registered lock modules keyed by `LockType`.
#[derive(Default)]
pub struct LockRegistry {
    modules: BTreeMap<LockType, Box<dyn LockModule>>,
}

impl LockRegistry {
    /// Empty registry. Use `with_first_party` for the shipping set.
    pub const fn new() -> Self {
        Self {
            modules: BTreeMap::new(),
        }
    }

    /// Registry with every first-party module Lantern ships in this plan.
    pub fn with_first_party() -> Self {
        let mut registry = Self::new();
        registry.register(Box::new(Secp256k1Lock));
        registry
    }

    /// Register or replace the module for its `lock_type`.
    pub fn register(&mut self, module: Box<dyn LockModule>) {
        self.modules.insert(module.lock_type(), module);
    }

    pub fn get(&self, lock_type: LockType) -> Result<&dyn LockModule, CoreError> {
        self.modules
            .get(&lock_type)
            .map(Box::as_ref)
            .ok_or(CoreError::UnsupportedLock(lock_type))
    }
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p lantern-wallet-core`
Expected: 15 tests pass. Each vault create or unlock runs Argon2id at 64 MiB, so this suite takes a few seconds.

- [ ] **Step 6: Clippy and commit**

Run: `cargo clippy -p lantern-wallet-core --all-targets -- -D warnings`
Expected: clean.

```bash
cargo fmt --all
git add crates/wallet-core
git commit -m "feat(wallet-core): Keyring over the vault, LockRegistry with first-party modules"
```

---

### Task 13: wallet-core — `WalletCore` and `SigningCoordinator`

**Files:**
- Create: `crates/wallet-core/src/core.rs`
- Modify: `crates/wallet-core/src/lib.rs`

**Interfaces:**
- Consumes: everything from Tasks 9 to 12
- Produces:
  - `ProfilePaths { vault: PathBuf, accounts: PathBuf }` with `fn in_dir(dir: &Path) -> Self`
  - `WalletCore::create(paths, password, network, words) -> Result<(Self, Phrase)>`, `import(paths, password, network, phrase) -> Result<Self>`, `unlock(paths, password, network) -> Result<Self>`, `with_locks(self, LockRegistry) -> Self`, `network(&self) -> Network`, `mnemonic_format(&self) -> Result<MnemonicFormat>`, `create_account(&mut self, label) -> Result<AccountRecord>`, `accounts(&self) -> Result<Vec<AccountRecord>>`, `reveal_mnemonic(&self, password) -> Result<Phrase>`, `lock(self)`, `signer(&self) -> SigningCoordinator<'_>`
  - `SigningCoordinator::sign_digest(&self, account_id: &str, digest: &[u8; 32]) -> Result<Vec<u8>, CoreError>`

- [ ] **Step 1: Write the failing tests**

Create `crates/wallet-core/src/core.rs`:

```rust
//! The orchestrator. Owns the unlocked vault, the account registry, the
//! lock modules, and the active network. `SigningCoordinator` is the one
//! signing entry point (spec §7); it dispatches through `LockModule` and
//! never names a scheme. Synchronous in plan 1c; plan 1e turns it into the
//! async `sign(tx)` when device signers and submission exist.

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use lantern_sdk_schema::{
        AccountCapabilities, Derivation, LockError, LockModule, LockType, Network, ScriptTemplate,
        SeedKind,
    };
    use tempfile::tempdir;

    use super::{ProfilePaths, WalletCore};
    use crate::error::CoreError;
    use crate::locks::LockRegistry;
    use crate::mnemonic::WordCount;

    struct FakeLock {
        kind: SeedKind,
        seen: Arc<Mutex<Vec<usize>>>,
    }

    impl LockModule for FakeLock {
        fn lock_type(&self) -> LockType {
            LockType::Secp256k1Blake160
        }
        fn extension_id(&self) -> &'static str {
            "test.fake"
        }
        fn capabilities(&self) -> AccountCapabilities {
            AccountCapabilities {
                can_sign: true,
                hardware: false,
            }
        }
        fn script_template(&self) -> ScriptTemplate {
            ScriptTemplate {
                code_hash: [0x11; 32],
                hash_type: 1,
            }
        }
        fn seed_kind(&self) -> SeedKind {
            self.kind
        }
        fn witness_lock_len(&self) -> usize {
            1
        }
        fn derive_lock_args(&self, seed: &[u8], d: &Derivation) -> Result<Vec<u8>, LockError> {
            self.seen.lock().expect("mutex").push(seed.len());
            let tag = u8::try_from(d.index).unwrap_or(u8::MAX);
            Ok(vec![tag; 20])
        }
        fn sign_digest(&self, seed: &[u8], _: &Derivation, digest: &[u8; 32]) -> Result<Vec<u8>, LockError> {
            self.seen.lock().expect("mutex").push(seed.len());
            Ok(digest.to_vec())
        }
    }

    fn fake_registry(kind: SeedKind) -> (LockRegistry, Arc<Mutex<Vec<usize>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut registry = LockRegistry::new();
        registry.register(Box::new(FakeLock {
            kind,
            seen: Arc::clone(&seen),
        }));
        (registry, seen)
    }

    #[test]
    fn coordinator_hands_raw_entropy_to_a_pq_shaped_module() {
        let dir = tempdir().expect("tempdir");
        let (locks, seen) = fake_registry(SeedKind::RawEntropy);
        let (core, _phrase) = WalletCore::create(
            ProfilePaths::in_dir(dir.path()),
            b"pw",
            Network::Testnet,
            WordCount::Words24,
        )
        .expect("creates");
        let mut core = core.with_locks(locks);
        let record = core.create_account("pq").expect("creates account");
        assert_eq!(record.extension_id, "test.fake");
        core.signer()
            .sign_digest(&record.id, &[0u8; 32])
            .expect("signs");
        assert_eq!(*seen.lock().expect("mutex"), vec![32, 32]);
    }

    #[test]
    fn coordinator_hands_the_bip39_seed_to_a_bip32_module() {
        let dir = tempdir().expect("tempdir");
        let (locks, seen) = fake_registry(SeedKind::Bip39Seed);
        let (core, _phrase) = WalletCore::create(
            ProfilePaths::in_dir(dir.path()),
            b"pw",
            Network::Testnet,
            WordCount::Words12,
        )
        .expect("creates");
        let mut core = core.with_locks(locks);
        core.create_account("hd").expect("creates account");
        assert_eq!(*seen.lock().expect("mutex"), vec![64]);
    }

    #[test]
    fn empty_lock_registry_is_unsupported_lock() {
        let dir = tempdir().expect("tempdir");
        let (core, _phrase) = WalletCore::create(
            ProfilePaths::in_dir(dir.path()),
            b"pw",
            Network::Testnet,
            WordCount::Words12,
        )
        .expect("creates");
        let mut core = core.with_locks(LockRegistry::new());
        assert!(matches!(
            core.create_account("x"),
            Err(CoreError::UnsupportedLock(LockType::Secp256k1Blake160))
        ));
    }
}
```

Add to `lib.rs`: `pub mod core;` and `pub use core::{ProfilePaths, SigningCoordinator, WalletCore};`.

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p lantern-wallet-core`
Expected: compile error, `WalletCore` not found.

- [ ] **Step 3: Write the implementation (above the test module in `core.rs`)**

```rust
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lantern_account_registry::{AccountRegistry, StoredAccount, account_id, to_record};
use lantern_sdk_schema::{AccountRecord, Derivation, LockType, Network};
use lantern_vault::{ExposeSecret, Vault};

use crate::error::CoreError;
use crate::keyring::Keyring;
use crate::locks::LockRegistry;
use crate::mnemonic::{MnemonicFormat, Phrase, WordCount};

/// Where a profile keeps its two files. Resolving the OS data directory
/// is the Tauri shell's job (plan 1f).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfilePaths {
    pub vault: PathBuf,
    pub accounts: PathBuf,
}

impl ProfilePaths {
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            vault: dir.join("vault.bin"),
            accounts: dir.join("accounts.json"),
        }
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// An unlocked profile.
pub struct WalletCore {
    vault: Vault,
    accounts: AccountRegistry,
    locks: LockRegistry,
    network: Network,
    paths: ProfilePaths,
}

impl WalletCore {
    /// Create a new profile. Refuses to overwrite an existing vault file.
    /// Returns the phrase exactly once for the show-and-confirm flow.
    pub fn create(
        paths: ProfilePaths,
        password: &[u8],
        network: Network,
        words: WordCount,
    ) -> Result<(Self, Phrase), CoreError> {
        if paths.vault.exists() {
            return Err(CoreError::AlreadyInitialised);
        }
        let mut vault = Vault::create(&paths.vault, password)?;
        let phrase = Keyring::create(&mut vault, words)?;
        let accounts = AccountRegistry::open(&paths.accounts)?;
        Ok((
            Self {
                vault,
                accounts,
                locks: LockRegistry::with_first_party(),
                network,
                paths,
            },
            phrase,
        ))
    }

    /// Create a new profile from an existing phrase (standard or combined).
    pub fn import(
        paths: ProfilePaths,
        password: &[u8],
        network: Network,
        phrase: &str,
    ) -> Result<Self, CoreError> {
        if paths.vault.exists() {
            return Err(CoreError::AlreadyInitialised);
        }
        let mut vault = Vault::create(&paths.vault, password)?;
        Keyring::import(&mut vault, phrase)?;
        let accounts = AccountRegistry::open(&paths.accounts)?;
        Ok(Self {
            vault,
            accounts,
            locks: LockRegistry::with_first_party(),
            network,
            paths,
        })
    }

    /// Open an existing profile.
    pub fn unlock(paths: ProfilePaths, password: &[u8], network: Network) -> Result<Self, CoreError> {
        let vault = Vault::unlock(&paths.vault, password)?;
        if !Keyring::is_initialised(&vault) {
            return Err(CoreError::SeedMissing);
        }
        let accounts = AccountRegistry::open(&paths.accounts)?;
        Ok(Self {
            vault,
            accounts,
            locks: LockRegistry::with_first_party(),
            network,
            paths,
        })
    }

    /// Replace the lock modules (tests, future extension host).
    pub fn with_locks(mut self, locks: LockRegistry) -> Self {
        self.locks = locks;
        self
    }

    pub const fn network(&self) -> Network {
        self.network
    }

    pub fn mnemonic_format(&self) -> Result<MnemonicFormat, CoreError> {
        Keyring::format(&self.vault)
    }

    /// Derive the next receiving account under the secp256k1 module.
    pub fn create_account(&mut self, label: &str) -> Result<AccountRecord, CoreError> {
        let module = self.locks.get(LockType::Secp256k1Blake160)?;
        let derivation = Derivation {
            change: 0,
            index: self.accounts.next_index(0),
        };
        let seed = Keyring::seed_for(&self.vault, module.seed_kind())?;
        let lock_args = module.derive_lock_args(seed.expose_secret(), &derivation)?;
        drop(seed);
        let stored = StoredAccount {
            id: account_id(module.lock_type(), &lock_args),
            label: label.to_string(),
            lock_type: module.lock_type(),
            extension_id: module.extension_id().to_string(),
            lock_args,
            derivation: Some(derivation),
            created_at: now_unix(),
        };
        let record = to_record(
            &stored,
            self.network,
            &module.script_template(),
            module.capabilities(),
        )?;
        self.accounts.add(stored)?;
        self.accounts.save()?;
        Ok(record)
    }

    /// Public projections of every account for the active network.
    pub fn accounts(&self) -> Result<Vec<AccountRecord>, CoreError> {
        self.accounts
            .list()
            .iter()
            .map(|account| {
                let module = self.locks.get(account.lock_type)?;
                to_record(
                    account,
                    self.network,
                    &module.script_template(),
                    module.capabilities(),
                )
                .map_err(CoreError::from)
            })
            .collect()
    }

    /// Re-prove the password against the vault file, then render the phrase.
    pub fn reveal_mnemonic(&self, password: &[u8]) -> Result<Phrase, CoreError> {
        let fresh = Vault::unlock(&self.paths.vault, password)?;
        let phrase = Keyring::phrase(&fresh)?;
        fresh.lock();
        Ok(phrase)
    }

    /// Zeroize the unlocked state.
    pub fn lock(self) {
        self.vault.lock();
    }

    pub const fn signer(&self) -> SigningCoordinator<'_> {
        SigningCoordinator { core: self }
    }
}

/// The single signing entry point.
pub struct SigningCoordinator<'a> {
    core: &'a WalletCore,
}

impl SigningCoordinator<'_> {
    /// Sign a 32-byte digest for `account_id`. Returns the witness lock bytes.
    pub fn sign_digest(&self, account_id: &str, digest: &[u8; 32]) -> Result<Vec<u8>, CoreError> {
        let account = self
            .core
            .accounts
            .get(account_id)
            .ok_or(CoreError::AccountNotFound)?;
        let module = self.core.locks.get(account.lock_type)?;
        let derivation = account.derivation.ok_or(CoreError::NoSigningMaterial)?;
        let seed = Keyring::seed_for(&self.core.vault, module.seed_kind())?;
        module
            .sign_digest(seed.expose_secret(), &derivation, digest)
            .map_err(CoreError::from)
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p lantern-wallet-core`
Expected: 18 tests pass.

- [ ] **Step 5: Clippy and commit**

Run: `cargo clippy -p lantern-wallet-core --all-targets -- -D warnings`
Expected: clean. If the nursery lint `significant_drop_tightening` complains about the `Mutex` guards in the test, wrap each `self.seen.lock().expect("mutex").push(..)` in its own block `{ ... }`.

```bash
cargo fmt --all
git add crates/wallet-core
git commit -m "feat(wallet-core): WalletCore profile lifecycle and LockModule-dispatched SigningCoordinator"
```

---

### Task 14: end-to-end test, workspace gate, docs, tag

**Files:**
- Create: `crates/wallet-core/tests/wallet_e2e.rs`
- Modify: `docs/superpowers/specs/2026-09-09-plan-1c-accounts-signing-design.md` (three refinements found while planning)
- Modify: `README.md` (status line)
- Modify: `~/.claude/rules/ckb-transactions.feedback.md` (one appended line)

**Interfaces:**
- Consumes: the whole public surface of `wallet-core` and `signer-secp256k1`.

- [ ] **Step 1: Write the end-to-end test**

Create `crates/wallet-core/tests/wallet_e2e.rs`:

```rust
//! End-to-end proof across all four crates. Every assertion that involves
//! key material cross-checks against `signer-secp256k1`, whose own tests
//! pin the lumos and BIP32 vectors.

use lantern_sdk_schema::{AccountRecord, Network};
use lantern_signer_secp256k1::{SigningKey, blake160, public_key, recover};
use lantern_vault::{Vault, VaultError};
use lantern_wallet_core::{CoreError, MnemonicFormat, ProfilePaths, WalletCore, WordCount};
use tempfile::tempdir;

const TANK: &str = "tank planet champion pottery together intact quick police asset flower sudden question";
const P1: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const P2: &str = "legal winner thank year wave sausage worth useful legal winner thank yellow";
const P3: &str = "letter advice cage absurd amount doctor acoustic avoid letter advice cage above";

fn lock_args_of(record: &AccountRecord) -> Vec<u8> {
    let hex_args = record.public_metadata["lockArgs"]
        .as_str()
        .expect("lockArgs is a string");
    hex::decode(hex_args.trim_start_matches("0x")).expect("hex")
}

#[test]
fn create_three_accounts_lock_unlock_sign_and_recover() {
    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    let (mut core, phrase) =
        WalletCore::create(paths.clone(), b"pw", Network::Testnet, WordCount::Words24)
            .expect("creates");
    assert_eq!(phrase.word_count(), 24);

    let a0 = core.create_account("One").expect("account 0");
    let a1 = core.create_account("Two").expect("account 1");
    let a2 = core.create_account("Three").expect("account 2");
    assert_eq!(a0.public_metadata["derivation"]["index"], 0);
    assert_eq!(a1.public_metadata["derivation"]["index"], 1);
    assert_eq!(a2.public_metadata["derivation"]["index"], 2);
    assert!(a0.address.starts_with("ckt1"), "{}", a0.address);
    assert_ne!(a0.id, a1.id);
    assert_ne!(lock_args_of(&a0), lock_args_of(&a1));
    core.lock();

    let core = WalletCore::unlock(paths, b"pw", Network::Testnet).expect("unlocks");
    let accounts = core.accounts().expect("lists");
    assert_eq!(accounts.len(), 3);
    assert_eq!(accounts[1], a1);

    let digest = [0x5au8; 32];
    let signature = core.signer().sign_digest(&a1.id, &digest).expect("signs");
    assert_eq!(signature.len(), 65);
    let mut arr = [0u8; 65];
    arr.copy_from_slice(&signature);
    let recovered = recover(&arr, &digest).expect("recovers");
    assert_eq!(blake160(&recovered).to_vec(), lock_args_of(&a1));

    assert!(matches!(
        core.signer().sign_digest("nope", &digest),
        Err(CoreError::AccountNotFound)
    ));
}

#[test]
fn imported_phrase_yields_the_lumos_key_and_reveals_with_the_password() {
    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    let mut core = WalletCore::import(paths, b"pw", Network::Testnet, TANK).expect("imports");
    assert_eq!(core.mnemonic_format().expect("format"), MnemonicFormat::Single);

    let account = core.create_account("Imported").expect("account");
    let key_bytes = hex::decode("848422863825f69e66dc7f48a3302459ec845395370c23578817456ad6b04b14")
        .expect("hex");
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&key_bytes);
    let expected = blake160(&public_key(&SigningKey::from_bytes(arr).expect("valid")));
    assert_eq!(lock_args_of(&account), expected.to_vec());
    assert_eq!(account.lock_type, lantern_sdk_schema::LockType::Secp256k1Blake160);
    assert_eq!(account.extension_id, "core.secp256k1");
    assert!(account.capabilities.can_sign);

    let again = core.reveal_mnemonic(b"pw").expect("reveals");
    assert_eq!(again.expose(), TANK);
    assert!(matches!(
        core.reveal_mnemonic(b"wrong"),
        Err(CoreError::Vault(VaultError::WrongPassword))
    ));
}

#[test]
fn quantum_purse_combined_phrase_imports_and_signs() {
    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    let combined = format!("{P1} {P2} {P3}");
    let mut core =
        WalletCore::import(paths, b"pw", Network::Mainnet, &combined).expect("imports");
    assert_eq!(core.mnemonic_format().expect("format"), MnemonicFormat::Combined3);

    let account = core.create_account("PQ import").expect("account");
    assert!(account.address.starts_with("ckb1"), "{}", account.address);
    assert_eq!(lock_args_of(&account).len(), 20);

    let digest = [0x77u8; 32];
    let signature = core.signer().sign_digest(&account.id, &digest).expect("signs");
    let mut arr = [0u8; 65];
    arr.copy_from_slice(&signature);
    let recovered = recover(&arr, &digest).expect("recovers");
    assert_eq!(blake160(&recovered).to_vec(), lock_args_of(&account));

    let again = core.reveal_mnemonic(b"pw").expect("reveals");
    assert_eq!(again.expose(), combined);
    assert_eq!(again.word_count(), 36);
}

#[test]
fn create_refuses_an_existing_vault_and_unlock_needs_a_seed() {
    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    {
        let (core, _phrase) =
            WalletCore::create(paths.clone(), b"pw", Network::Testnet, WordCount::Words12)
                .expect("creates");
        core.lock();
    }
    assert!(matches!(
        WalletCore::create(paths.clone(), b"pw", Network::Testnet, WordCount::Words12),
        Err(CoreError::AlreadyInitialised)
    ));
    assert!(matches!(
        WalletCore::import(paths, b"pw", Network::Testnet, TANK),
        Err(CoreError::AlreadyInitialised)
    ));

    let empty = tempdir().expect("tempdir");
    let empty_paths = ProfilePaths::in_dir(empty.path());
    Vault::create(&empty_paths.vault, b"pw").expect("bare vault");
    assert!(matches!(
        WalletCore::unlock(empty_paths, b"pw", Network::Testnet),
        Err(CoreError::SeedMissing)
    ));
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p lantern-wallet-core --test wallet_e2e`
Expected: 4 tests pass. Roughly ten Argon2id runs in total, so allow several seconds.

- [ ] **Step 3: Whole-workspace gate**

Run, in order:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p lantern-vault --all-targets --no-default-features -- -D warnings
cargo test --workspace
cargo test -p lantern-vault --no-default-features
cargo doc --workspace --no-deps
```

Expected: all clean. `cargo doc` must not warn about broken intra-doc links; the `doc_markdown` lint has already forced backticks around identifiers.

- [ ] **Step 4: Record the three spec refinements**

Edit `docs/superpowers/specs/2026-09-09-plan-1c-accounts-signing-design.md`:

1. In §4.3, the `public_metadata` sentence: replace `"derivationPath": "m/44'/309'/0'/0/i"` with `"derivation": { "change": 0, "index": i }` and add: "The path string is lock-specific, so the registry stores the generic pair and the UI renders the path."
2. In §4.4, `SigningCoordinator::sign_digest` returns `Vec<u8>` (the witness lock bytes from the module), not `[u8; 65]`, and `WalletCore` gains `mnemonic_format()` and `with_locks()`.
3. In §7, add `NoSigningMaterial` to the wallet-core row (an account without a `Derivation`, which watch-only accounts will be).

- [ ] **Step 5: Update the README status line**

In `README.md`, replace `**Status:** Foundation design complete (v1.1). Implementation about to begin.` with:

```markdown
**Status:** Plans 1a (scaffold), 1b (vault) and 1c (accounts + secp256k1 signing) complete. Next: plan 1d (chain backend).
```

- [ ] **Step 6: Append the feedback-log line**

Append one line to `~/.claude/rules/ckb-transactions.feedback.md` under `## Entries`, using today's date, of the shape:

```
2026-MM-DD | ckb-wallet plan 1c (accounts + secp signer, no broadcast) | HIT §12 NEUTRAL §1-§11 | <one sentence: which vectors held first try, which trap bit, whether the on-chain sighash vector in signer/sighash.rs needed a fix>
```

- [ ] **Step 7: Commit and tag**

```bash
cargo fmt --all
git add crates/wallet-core/tests/wallet_e2e.rs docs/superpowers/specs/2026-09-09-plan-1c-accounts-signing-design.md README.md
git commit -m "test(wallet-core): end-to-end create/import/sign/reveal across all plan 1c crates"
git tag -a v0.0.3-signing -m "Plan 1c: accounts, secp256k1 signing, sdk schema, mlock"
```

Do not push the tag; Phill pushes after review.

---

## Self-Review

**Spec coverage.** §1 goals 1 to 8 map to Tasks 11/12 (create, import, combined), 5/7 (derivation), 9/13 (records, addresses), 13 (single signing entry through the trait), 6 (sighash), 2 (TS export), 10 (mlock). §4.1 to §4.5 are Tasks 2, 3 to 7, 8 to 9, 11 to 13, 10 respectively. §5 persistence is Task 9 plus `ProfilePaths` in Task 13. §6 memory hygiene: `SigningKey`/`ExtendedKey` zeroize (Tasks 3, 5), `Phrase` over `SecretString` (Task 11), `seed_for` hands one kind only (Task 12, tested in Task 13), mlock (Task 10). §7 errors: Tasks 2, 3, 8, 11. §8 oracle table: every row has a test in Tasks 3, 5, 6, 8, 9, 10, 11, 13, 14. §9 task order matches. §10 open items untouched.

**Placeholders.** None. The only conditional instructions ("if clippy flags…") name the exact edit to make.

**Type consistency.** `Branch` replaces the spec's `Change` enum name (a variant named `Change` inside an enum named `Change` trips clippy); `Derivation.change: u32` is unchanged and `Branch::try_from` maps it. `derive_ckb_key` takes `&[u8]` (16..=64) rather than `&[u8; 64]` so the BIP32 and lumos 16-byte-seed vectors can exercise it; `Secp256k1Lock` enforces exactly 64. `Vault::get` returns `SecretSlice<u8>` everywhere it is consumed (Tasks 12, 13, e2e). `sign_digest` returns `Vec<u8>` at both the trait and the coordinator.

**Known caveats for the executor.**
- `specta` rc.20 attribute pass-through is the one API this plan could not verify offline; Task 2 Step 7 gives the fallback.
- libsecp256k1's `SecretKey` is `Copy`; transient stack copies are erased with `non_secure_erase` but compiler-made copies are not controllable. Documented in `key.rs`.
- The mlock tests assert the best-effort contract ("locked, or warned"), never a hard count, so they pass on hosts with a 64 KiB `RLIMIT_MEMLOCK`.
