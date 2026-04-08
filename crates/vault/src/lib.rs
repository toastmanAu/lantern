//! Lantern vault.
//!
//! Encrypted at-rest storage for secret material (mnemonic seeds, private keys,
//! per-extension secrets). Encryption: XChaCha20-Poly1305 + Argon2id KDF.
//! Implementation lands in plan 1b.

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    #[test]
    const fn placeholder() {}
}
