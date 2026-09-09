# Lantern Plan 1c — Accounts, secp256k1 Signing, SDK Schema, mlock (Design)

**Status:** approved design, 2026-09-09. Implementation plan follows in `docs/superpowers/plans/`.
**Parent spec:** `2026-04-08-foundation-design-tauri-edition-v1.1.md` §7 (Vault, Accounts, Signing), §9 (schema pipeline), §21 (backup and recovery), §22 (dependencies).
**Builds on:** plan 1a (scaffold, `v0.0.1-scaffold`) and plan 1b (vault, `v0.0.2-vault`).

---

## 1. Goal

After plan 1c a Lantern profile can:

1. Create a wallet from a fresh BIP39 mnemonic (12 or 24 words) or import an existing one.
2. Derive any number of `secp256k1_blake160` accounts on the Neuron-compatible path `m/44'/309'/0'/0/i`.
3. List those accounts as public `AccountRecord`s, with a CKB address rendered for the active network, without exposing any secret.
4. Produce a 65-byte recoverable signature over a 32-byte digest through one signing entry point, with the seed living in the vault and every derived key zeroized after use.
5. Compute the RFC 0019 `sighash_all` digest from a transaction hash and witness bytes, verified against ckb-sdk-rust.
6. Export the IPC types to TypeScript through `specta`, proving the schema pipeline from spec §9 works before any Tauri command exists.
7. Page-lock long-lived unlocked vault state on platforms that allow it.

## 2. Non-goals

- Transaction construction, fee estimation, witness placement, broadcast. Plan 1e (`tx-builder`).
- Any chain access. Plan 1d (`chain-backend`).
- Tauri commands, the `tauri-specta` builder, writing `bindings.ts` into `packages/extension-sdk`. Plan 1f.
- The `LockModule` trait. It is introduced with the second lock module so it is designed against two implementors.
- Change addresses (`m/44'/309'/0'/1/i`). The derivation function accepts the change branch; the account creation flow only uses the receiving branch in v0.1.
- BIP39 passphrase. The seed is derived with an empty passphrase. Adding one later is a new keyring blob, not a migration.
- Neuron keystore import (scrypt + AES-128-CTR JSON). Neuron **seed** import works by construction because the path matches.
- Multi-wallet per profile. Spec §7 says one vault per profile; plan 1c stores one seed per vault.

## 3. Decisions locked in this design

| Decision | Choice | Why |
|---|---|---|
| Key model | Full HD: BIP39 mnemonic, BIP32 derivation | Spec §21 already mandates it; Neuron seed compatibility from day one |
| Registry storage | Plaintext `accounts.json` beside the vault, public data only | Home screen can list accounts before unlock; SQLite waits for plan 1d where chain cache needs it |
| mlock | `region` 4.0 behind a default-on `mlock` feature, best-effort | `region::lock` is a safe fn so the workspace `unsafe_code = "forbid"` stays; a hard failure would lock out Linux users with the default 64 KiB `RLIMIT_MEMLOCK` |
| Curve implementation | `secp256k1` 0.33 (libsecp256k1 bindings) | Same library as the CKB node and ckb-sdk-rust; constant-time C; native recoverable signatures |
| BIP32 | Hand-rolled CKD in `signer-secp256k1` | The published `bip32` 0.5.3 crate derives only on `k256`; pulling a second curve for 60 lines of HMAC plus `add_tweak` is not worth it |
| Signer scope | Digest signing plus `sighash_all` hashing over caller-supplied witness bytes | CKB-specific and testable against a known vector, without a molecule dependency |
| Seed lifecycle owner | `wallet-core` (`Keyring`) | Mnemonic handling is curve-independent; the signer stays pure math; matches spec §7 layering |
| Vault blob | BIP39 **entropy**, not the seed | Entropy regenerates both the phrase (for the password-gated reveal in §21) and the seed; a stored seed cannot be turned back into words |
| Account id | `"<lock_type>-<hex lock args>"` | Deterministic; re-importing the same seed cannot duplicate accounts |
| Coordinator signature | Synchronous `sign_digest` in 1c | Nothing awaits yet; clippy pedantic flags `unused_async`. Becomes the async `sign(tx)` of spec §7 in plan 1e when device signers and submission exist |

## 4. Crate map

Dependency direction is strictly downward. No cycles.

```
wallet-core ──► vault
            ──► account-registry ──► sdk-schema
            ──► signer-secp256k1
            ──► sdk-schema
```

### 4.1 `lantern-sdk-schema`

IPC types. Every type derives `specta::Type`, `serde::Serialize`, `serde::Deserialize` with `#[serde(rename_all = "camelCase")]`.

```rust
pub enum Network { Mainnet, Testnet }
impl Network { pub const fn address_prefix(self) -> &'static str }  // "ckb" | "ckt"

pub enum LockType { Secp256k1Blake160 }                             // snake_case on the wire

pub struct AccountCapabilities { pub can_sign: bool, pub hardware: bool }

pub struct AccountRecord {                                           // verbatim from spec §7
    pub id: String,
    pub label: String,
    pub lock_type: LockType,
    pub extension_id: String,
    pub address: String,
    pub public_metadata: serde_json::Value,
    pub capabilities: AccountCapabilities,
}

pub fn typescript_bindings() -> Result<String, SchemaError>          // renders all of the above via specta_typescript
```

`typescript_bindings` is the seam plan 1f's `tauri-specta::Builder` replaces. In 1c a unit test asserts the rendered TypeScript contains `lockType`, `extensionId`, `publicMetadata`, `canSign` and the `"secp256k1_blake160"` literal, proving the camelCase and enum wire shapes.

New dependencies: `specta-typescript` (already pinned in the workspace), `serde_json`.

### 4.2 `lantern-signer-secp256k1`

Stateless curve math. No storage, no I/O, no knowledge of the vault or registry.

```rust
pub struct SigningKey([u8; 32]);        // Zeroize + ZeroizeOnDrop, no Debug/Display
pub struct PublicKey([u8; 33]);         // compressed SEC1

pub const CKB_COIN_TYPE: u32 = 309;
pub enum Change { Receiving = 0, Change = 1 }

pub fn derive_ckb_key(seed: &[u8; 64], change: Change, index: u32) -> Result<SigningKey, SignerError>
pub fn public_key(key: &SigningKey) -> PublicKey
pub fn blake160(pubkey: &PublicKey) -> [u8; 20]
pub fn sign_recoverable(key: &SigningKey, digest: &[u8; 32]) -> [u8; 65]      // r ‖ s ‖ recid
pub fn recover(signature: &[u8; 65], digest: &[u8; 32]) -> Result<PublicKey, SignerError>
pub fn sighash_all(tx_hash: &[u8; 32], first_witness: &[u8], others: &[&[u8]]) -> [u8; 32]
```

**Derivation.** Master key and chain code are `HMAC-SHA512(key = "Bitcoin seed", data = seed)`. Path `m/44'/309'/0'/change/index` with the first three levels hardened (`i + 2^31`). Hardened child data is `0x00 ‖ k_par ‖ ser32(i)`; normal child data is `serP(K_par) ‖ ser32(i)`. The child key is `parse256(I_L) + k_par mod n`, computed by `secp256k1::SecretKey::add_tweak`. Intermediate keys and chain codes are zeroized on every level.

**Hashing.** `blake160` is the first 20 bytes of `ckb_hash::blake2b_256` (personalisation `ckb-default-hash`) over the 33-byte compressed public key. `ckb-hash` is the consensus implementation; the plan does not re-implement it.

**Signature layout.** `sign_recoverable` serialises the compact 64-byte signature followed by the recovery id as one byte, which is what the `secp256k1_blake160_sighash_all` lock reads.

**Sighash.** Exactly ckb-sdk-rust `generate_message`: `blake2b_256(tx_hash ‖ u64le(len(w0)) ‖ w0 ‖ Σ (u64le(len(wi)) ‖ wi))`, where `w0` is the first witness of the script group **with its lock field already replaced by 65 zero bytes by the caller**, and `others` are the remaining witnesses of the group followed by every witness beyond the input count. The signer takes bytes, so the caller (plan 1e) owns the molecule work.

New dependencies: `secp256k1` 0.33 (`recovery`, `rand`), `hmac`, `sha2`, `ckb-hash` 1.1, `zeroize`.

### 4.3 `lantern-account-registry`

Public-only storage.

```rust
pub struct Derivation { pub change: u32, pub index: u32 }

pub struct StoredAccount {                 // what accounts.json holds
    pub id: String,
    pub label: String,
    pub lock_type: LockType,
    pub extension_id: String,
    pub lock_args: Vec<u8>,                // hex on disk
    pub derivation: Option<Derivation>,    // None for imported/watch-only later
    pub created_at: u64,                   // unix seconds
}

pub struct AccountRegistry { /* path + Vec<StoredAccount> */ }
impl AccountRegistry {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RegistryError>   // missing file = empty registry
    pub fn add(&mut self, account: StoredAccount) -> Result<(), RegistryError>   // DuplicateId if present
    pub fn list(&self) -> &[StoredAccount]
    pub fn get(&self, id: &str) -> Option<&StoredAccount>
    pub fn set_label(&mut self, id: &str, label: &str) -> Result<(), RegistryError>
    pub fn remove(&mut self, id: &str) -> Result<StoredAccount, RegistryError>
    pub fn next_index(&self, change: u32) -> u32                            // max derived index + 1, or 0
    pub fn save(&self) -> Result<(), RegistryError>
}

pub fn to_record(account: &StoredAccount, network: Network) -> AccountRecord
pub mod address {
    pub const SECP256K1_BLAKE160_CODE_HASH: [u8; 32];   // 0x9bd7e06f…3cce8, identical on mainnet and testnet
    pub fn encode_full(network: Network, code_hash: &[u8; 32], hash_type: u8, args: &[u8]) -> String
}
```

**Address.** CKB2021 full format: payload `0x00 ‖ code_hash ‖ hash_type ‖ args`, bech32m, hrp `ckb` or `ckt`. Hash type for the system secp lock is `type` (`0x01`).

**Projection.** `to_record` computes `address` at read time so one stored record serves both networks. `public_metadata` carries `{ "derivationPath": "m/44'/309'/0'/0/i", "lockArgs": "0x…" }`. `capabilities` is `{ can_sign: true, hardware: false }` for this lock type.

**File.** `accounts.json` is `{ "version": 1, "accounts": [...] }`. Writes go to `accounts.json.tmp` then `fs::rename`. A parse failure is `RegistryError::Corrupt` and is never silently replaced with an empty registry.

New dependencies: `lantern-sdk-schema`, `serde_json`, `bech32` 0.12, `hex`.

### 4.4 `lantern-wallet-core`

Orchestration and the single signing entry point.

```rust
pub struct ProfilePaths { pub vault: PathBuf, pub accounts: PathBuf }

pub struct Keyring;                        // namespace for the vault blob "keyring/bip39-entropy"
impl Keyring {
    pub fn create(vault: &mut Vault, words: WordCount) -> Result<Mnemonic, CoreError>   // AlreadyInitialised if blob exists
    pub fn import(vault: &mut Vault, phrase: &str) -> Result<(), CoreError>           // InvalidMnemonic on bad checksum/word
    pub fn is_initialised(vault: &Vault) -> bool
    pub fn seed(vault: &Vault) -> Result<SecretBox<[u8; 64]>, CoreError>              // SeedMissing if blob absent
    pub fn mnemonic(vault: &Vault) -> Result<Mnemonic, CoreError>
}

pub struct WalletCore { vault: Vault, registry: AccountRegistry, network: Network, paths: ProfilePaths }
impl WalletCore {
    pub fn create(paths, password, network, words) -> Result<(Self, Mnemonic), CoreError>
    pub fn import(paths, password, network, phrase) -> Result<Self, CoreError>
    pub fn unlock(paths, password, network) -> Result<Self, CoreError>
    pub fn create_account(&mut self, label: &str) -> Result<AccountRecord, CoreError>
    pub fn accounts(&self) -> Vec<AccountRecord>
    pub fn reveal_mnemonic(&self, password: &[u8]) -> Result<Mnemonic, CoreError>     // re-unlocks vault.bin with the password; WrongPassword otherwise
    pub fn lock(self)
    pub fn signer(&self) -> SigningCoordinator<'_>
}

pub struct SigningCoordinator<'a> { /* &WalletCore */ }
impl SigningCoordinator<'_> {
    pub fn sign_digest(&self, account_id: &str, digest: &[u8; 32]) -> Result<[u8; 65], CoreError>
}
```

`sign_digest` resolves the record, rejects anything but `Secp256k1Blake160` with `UnsupportedLock`, regenerates the seed from entropy, derives the key at the stored `Derivation`, signs, and lets `SigningKey` and the seed `SecretBox` drop. `create_account` uses `registry.next_index(0)`, derives the public key, stores the record, saves the registry, and returns the projection. `Mnemonic` is the `bip39` type with its `zeroize` feature; `WalletCore::create` returns it exactly once for the show-and-confirm flow in spec §21.

New dependencies: `lantern-vault`, `lantern-account-registry`, `lantern-signer-secp256k1`, `lantern-sdk-schema`, `bip39` 2.2 (`rand`, `zeroize`), `secrecy`, `zeroize`.

### 4.5 `lantern-vault` changes

1. `Vault::get` returns `Option<SecretSlice<u8>>` instead of `Option<Vec<u8>>`. Callers must `expose_secret()`. The only current callers are the vault's own tests.
2. New feature `mlock` (default on) adds `region` 4.0. On `unlock` and `create`, and after every `put`, the vault locks the master key box and each blob's backing buffer and keeps the `LockGuard`s in a `Vec` inside `Vault`. Guards drop on `lock()` or `Drop`. A `region::Error` logs one `tracing::warn!` per vault session and continues. With the feature off the locking calls compile to nothing.
3. Module docs state what is and is not locked: long-lived unlocked state is; transient Argon2 output and the decrypted CBOR buffer are zeroized but not locked.
4. The `fs::rename` in `save` gets a comment noting Windows fails when the destination exists, matching the registry.
5. CI adds `cargo test -p lantern-vault --no-default-features`.

## 5. Persistence layout

```
<profile dir>/
├── vault.bin        # plan 1b format, unchanged; holds "keyring/bip39-entropy"
└── accounts.json    # { "version": 1, "accounts": [StoredAccount…] }, public data only
```

Wallet-core receives `ProfilePaths` explicitly. Resolving the OS data directory (`~/.config/lantern/profiles/<name>/`) is Tauri's job in plan 1f. Tests use `tempfile::tempdir()`.

## 6. Memory hygiene

- Entropy leaves the vault only as `SecretSlice<u8>`; the seed only as `SecretBox<[u8; 64]>`; derived keys only as `SigningKey` (zeroize on drop). None implement `Debug`, `Display`, `Clone`, or `Serialize`.
- BIP32 intermediates (`I`, `I_L`, `I_R`, per-level chain codes) are zeroized at the end of each level.
- `Mnemonic` uses the `bip39` crate's `zeroize` feature. `WalletCore::create` hands it to the caller once; the caller is responsible for dropping it after confirmation.
- `reveal_mnemonic` requires the password again and proves it by re-running the vault unlock from disk (Argon2id, roughly one second). No cached password, no "already unlocked" shortcut.
- mlock covers the master key and inner-store blobs as described in §4.5.

## 7. Error handling

One `thiserror` enum per crate. `Display` text never contains key bytes, phrase words, file contents, or paths beyond the file name.

| Crate | Variants |
|---|---|
| sdk-schema | `Export(String)` |
| signer-secp256k1 | `InvalidKey`, `InvalidSignature`, `DerivationOverflow` (index ≥ 2^31 on the non-hardened levels) |
| account-registry | `Io`, `Corrupt`, `DuplicateId`, `NotFound`, `Address` |
| wallet-core | `Vault(VaultError)`, `Registry(RegistryError)`, `Signer(SignerError)`, `AlreadyInitialised`, `SeedMissing`, `InvalidMnemonic`, `AccountNotFound`, `UnsupportedLock` |

`InvalidMnemonic` deliberately drops the `bip39` error detail, which can echo the offending word.

## 8. Testing

Every cryptographic claim is pinned to an external oracle, never to the implementation's own output.

| Area | Oracle |
|---|---|
| BIP39 entropy → phrase → seed | Trezor vectors from `research/ckb-tx-construction/raw/lumos/packages/hd/tests/mnemonic/fixtures.json` |
| BIP32 CKD | BIP32 reference test vector 1 (`m/0'/1/2'/2/1000000000`) plus the lumos keychain vectors on `m/44'/309'/0'` |
| blake160 and address | pubkey-to-address vectors from ckb-sdk-rust and lumos tests |
| `sighash_all` | one vector produced by running ckb-sdk-rust `generate_message` in the scratchpad, pinned in the test with the exact inputs |
| `sign_recoverable` | recover the public key with libsecp256k1 and compare; also verify with the non-recoverable verifier |
| TypeScript export | rendered string contains the expected camelCase names and enum literal |
| Registry | JSON round trip, corrupt file → `Corrupt`, duplicate id → `DuplicateId`, `next_index` with gaps |
| Vault | existing 34 tests pass with and without `mlock`; `get` returns `SecretSlice` |
| End to end (wallet-core integration test) | create → three accounts → lock → unlock → `sign_digest` → recovered pubkey equals the account's; import a known phrase → the known Neuron/lumos address; `reveal_mnemonic` with the wrong password → `WrongPassword`; second `create` on the same vault → `AlreadyInitialised` |

Negative tests follow the disable-and-re-enable discipline from the ckb-transactions feedback log: each assertion must fail when its guard is commented out.

No transaction is broadcast in plan 1c. On-chain proof of the signature path lands in plan 1e together with the first testnet transfer.

## 9. Task order for the implementation plan

1. Wire workspace and crate `Cargo.toml` dependencies; bump workspace version to 0.0.3.
2. sdk-schema types plus `typescript_bindings` test.
3. signer: `SigningKey`, `public_key`, `blake160`.
4. signer: `sign_recoverable`, `recover`.
5. signer: BIP32 `derive_ckb_key`.
6. signer: `sighash_all`.
7. registry: `address` module.
8. registry: `StoredAccount`, `AccountRegistry`, `to_record`, persistence.
9. vault: `SecretSlice` return type, `mlock` feature, CI matrix entry.
10. wallet-core: `Keyring`.
11. wallet-core: `WalletCore` and `SigningCoordinator`.
12. Integration test, clippy and doc pass, tag `v0.0.3-signing`.

## 10. Open items carried forward (not blocking 1c)

- `LockModule` trait: with the watch-only or Ledger module.
- Change-branch accounts and gap-limit recovery scan (spec §21): plan 1d, needs chain access.
- Vault header authentication (`TODO(v2)` in `aead.rs`): next vault format bump.
- Neuron keystore JSON import: after 1e, with the import UI.
- Deferred cargo upgrade sweep (rand 0.9, sha2 0.11, hkdf 0.13): separate PR, unchanged.
