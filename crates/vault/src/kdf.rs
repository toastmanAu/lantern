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
    let params = Params::new(m_cost_kib, u32::from(t_cost), u32::from(p_cost), Some(32))
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
    use crate::error::VaultError;
    use crate::format::{V1_ARGON2_M_KIB, V1_ARGON2_P, V1_ARGON2_T};

    // Use tiny params in tests so they don't take 400ms each.
    const TEST_M: u32 = 8; // 8 KiB
    const TEST_T: u16 = 1;
    const TEST_P: u16 = 1;

    #[test]
    fn same_inputs_same_output() {
        let k1 =
            derive_master_key(b"hunter2", b"salty-salty-saltsa", TEST_M, TEST_T, TEST_P).unwrap();
        let k2 =
            derive_master_key(b"hunter2", b"salty-salty-saltsa", TEST_M, TEST_T, TEST_P).unwrap();
        assert_eq!(k1, k2);
    }

    #[test]
    fn different_passwords_diverge() {
        let k1 =
            derive_master_key(b"hunter2", b"salty-salty-saltsa", TEST_M, TEST_T, TEST_P).unwrap();
        let k2 =
            derive_master_key(b"hunter3", b"salty-salty-saltsa", TEST_M, TEST_T, TEST_P).unwrap();
        assert_ne!(k1, k2);
    }

    #[test]
    fn different_salts_diverge() {
        let k1 =
            derive_master_key(b"hunter2", b"salty-salty-salts1", TEST_M, TEST_T, TEST_P).unwrap();
        let k2 =
            derive_master_key(b"hunter2", b"salty-salty-salts2", TEST_M, TEST_T, TEST_P).unwrap();
        assert_ne!(k1, k2);
    }

    #[test]
    fn invalid_params_return_kdf_failed() {
        // m_cost_kib = 0 is rejected by Params::new, must surface as KdfFailed.
        let err = derive_master_key(b"x", &[0u8; 16], 0, 1, 1).unwrap_err();
        assert!(matches!(err, VaultError::KdfFailed));
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
