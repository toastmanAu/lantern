//! Lantern post-quantum signer.
//!
//! ML-DSA-44/65/87 (FIPS 204) and Falcon-512/1024 signing for accounts
//! protected by the ckb-mldsa-lock family. Implementation lands in plan 5.

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    #[test]
    const fn placeholder() {}
}
