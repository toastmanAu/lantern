//! Lantern passkey signer.
//!
//! `WebAuthn` / FIDO2 / CTAP2 hardware key support. v0.1 baseline is hardware
//! FIDO2 keys via libfido2 on all three platforms; biometric paths added
//! per-platform where available. Implementation lands in plan 4.

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    #[test]
    const fn placeholder() {}
}
