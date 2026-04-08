//! Lantern `secp256k1_blake160` signer.
//!
//! Implements the canonical CKB `secp256k1_blake160` sighash signing scheme
//! per RFC 0019. Witness layout follows the `WitnessArgs` molecule schema.
//! Implementation lands in plan 1c.

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    #[test]
    const fn placeholder() {}
}
