#![forbid(unsafe_code)]

pub mod error;
pub mod format;
pub mod kdf;
pub mod aead;
pub mod secret;
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
