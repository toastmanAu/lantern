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
