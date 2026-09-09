//! Seed lifecycle over the vault. One blob, `keyring/entropy`, holds the
//! BIP39 entropy (16..=32 bytes single, 48/72/96 combined). Everything
//! else (phrase, BIP39 seed) is regenerated on demand.

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
    /// Whether the vault already holds seed entropy.
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

    /// Copy of the stored entropy, or `SeedMissing` if the vault is empty.
    pub fn entropy(vault: &Vault) -> Result<SecretSlice<u8>, CoreError> {
        vault.get(ENTROPY_BLOB).ok_or(CoreError::SeedMissing)
    }

    /// Which mnemonic format the stored entropy renders as.
    pub fn format(vault: &Vault) -> Result<MnemonicFormat, CoreError> {
        let entropy = Self::entropy(vault)?;
        format_for_entropy(entropy.expose_secret().len())
    }

    /// The canonical phrase text for the stored entropy.
    pub fn phrase(vault: &Vault) -> Result<Phrase, CoreError> {
        let entropy = Self::entropy(vault)?;
        render_phrase(entropy.expose_secret())
    }

    /// The 64-byte BIP39 seed derived from the stored entropy.
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

#[cfg(test)]
mod tests {
    use lantern_sdk_schema::SeedKind;
    use lantern_vault::{ExposeSecret, Vault};
    use tempfile::tempdir;

    use super::{ENTROPY_BLOB, Keyring};
    use crate::error::CoreError;
    use crate::mnemonic::{MnemonicFormat, WordCount};

    const TANK: &str =
        "tank planet champion pottery together intact quick police asset flower sudden question";

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
        assert_eq!(
            Keyring::entropy(&vault)
                .expect("entropy")
                .expose_secret()
                .len(),
            32
        );
        assert_eq!(
            Keyring::format(&vault).expect("format"),
            MnemonicFormat::Single
        );
        assert_eq!(
            Keyring::phrase(&vault).expect("phrase").expose(),
            phrase_text
        );
    }

    #[test]
    fn import_known_phrase_yields_known_seed() {
        let dir = tempdir().expect("tempdir");
        let mut vault = Vault::create(dir.path().join("vault.bin"), b"pw").expect("creates");
        assert_eq!(
            Keyring::import(&mut vault, TANK).expect("imports"),
            MnemonicFormat::Single
        );
        let seed = Keyring::bip39_seed(&vault).expect("seed");
        assert_eq!(
            hex::encode(seed.expose_secret()),
            "1371018cfad5990f5e451bf586d59c3820a8671162d8700533549b0df61a63330e5cd5099a5d3938f833d51e4572104868bfac7cfe5b4063b1509a995652bc08"
        );
        assert!(matches!(
            Keyring::import(&mut vault, TANK),
            Err(CoreError::AlreadyInitialised)
        ));
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
        assert!(matches!(
            Keyring::entropy(&vault),
            Err(CoreError::SeedMissing)
        ));
        assert!(matches!(
            Keyring::phrase(&vault),
            Err(CoreError::SeedMissing)
        ));
    }
}
