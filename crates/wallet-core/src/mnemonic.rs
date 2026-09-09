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
    // Every 4 bytes of entropy yields 3 BIP39 words (32-bit checksum-carrying
    // groups), and the longest English wordlist entry is 8 characters, so a
    // 9-byte-per-word budget (8 + a separator) can never be exceeded and the
    // buffer never reallocates, leaving no un-zeroized prefix behind.
    let word_count = entropy.len() / 4 * 3;
    let mut text = Zeroizing::new(String::with_capacity(word_count * 9));
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
    pbkdf2::pbkdf2_hmac::<Sha512>(
        phrase.expose().as_bytes(),
        PBKDF2_SALT,
        PBKDF2_ROUNDS,
        &mut seed,
    );
    let boxed = SecretBox::new(Box::new(seed));
    seed.zeroize();
    Ok(boxed)
}

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
    const P3: &str =
        "letter advice cage absurd amount doctor acoustic avoid letter advice cage above";
    const P4: &str = "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo vote";
    const TANK: &str =
        "tank planet champion pottery together intact quick police asset flower sudden question";

    fn trezor_vectors() -> [(Vec<u8>, &'static str, &'static str); 4] {
        [
            (
                vec![0x00; 16],
                P1,
                "5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc19a5ac40b389cd370d086206dec8aa6c43daea6690f20ad3d8d48b2d2ce9e38e4",
            ),
            (
                vec![0x7f; 16],
                P2,
                "878386efb78845b3355bd15ea4d39ef97d179cb712b77d5c12b6be415fffeffe5f377ba02bf3f8544ab800b955e51fbff09828f682052a20faa6addbbddfb096",
            ),
            (
                vec![0x80; 16],
                P3,
                "77d6be9708c8218738934f84bbbb78a2e048ca007746cb764f0673e4b1812d176bbb173e1a291f31cf633f1d0bad7d3cf071c30e98cd0688b5bcce65ecaceb36",
            ),
            (
                vec![0xff; 32],
                P4,
                "e28a37058c7f5112ec9e16a3437cf363a2572d70b6ceb3b6965447623d620f14d06bb321a26b33ec15fcd84a3b5ddfd5520e230c924c87aaa0d559749e044fef",
            ),
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
        assert_eq!(
            format_for_entropy(entropy.len()).expect("format"),
            MnemonicFormat::Combined3
        );
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
        assert!(matches!(
            parse_phrase(&thirteen),
            Err(CoreError::InvalidMnemonic)
        ));
        let thirty = format!("{P1} {P2} legal winner thank year wave sausage");
        assert!(matches!(
            parse_phrase(&thirty),
            Err(CoreError::InvalidMnemonic)
        ));
        let forty_eight = format!("{P1} {P1} {P1} {P1}");
        assert!(matches!(
            parse_phrase(&forty_eight),
            Err(CoreError::InvalidMnemonic)
        ));
        let bad_checksum = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon";
        assert!(matches!(
            parse_phrase(bad_checksum),
            Err(CoreError::InvalidMnemonic)
        ));
        let unknown_word = P1.replace("about", "lantern");
        assert!(matches!(
            parse_phrase(&unknown_word),
            Err(CoreError::InvalidMnemonic)
        ));
        // A combined phrase whose second chunk has a bad checksum fails as a whole.
        let bad_middle = format!("{P1} {bad_checksum} {P3}");
        assert!(matches!(
            parse_phrase(&bad_middle),
            Err(CoreError::InvalidMnemonic)
        ));
    }

    #[test]
    fn entropy_length_discriminates_format() {
        assert_eq!(format_for_entropy(16).expect("ok"), MnemonicFormat::Single);
        assert_eq!(format_for_entropy(20).expect("ok"), MnemonicFormat::Single);
        assert_eq!(format_for_entropy(32).expect("ok"), MnemonicFormat::Single);
        assert_eq!(
            format_for_entropy(48).expect("ok"),
            MnemonicFormat::Combined3
        );
        assert_eq!(
            format_for_entropy(72).expect("ok"),
            MnemonicFormat::Combined3
        );
        assert_eq!(
            format_for_entropy(96).expect("ok"),
            MnemonicFormat::Combined3
        );
        assert!(matches!(
            format_for_entropy(40),
            Err(CoreError::InvalidMnemonic)
        ));
        assert!(matches!(
            render_phrase(&[0u8; 40]),
            Err(CoreError::InvalidMnemonic)
        ));
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
    fn render_phrase_capacity_bound_holds_for_96_byte_entropy() {
        // Sanity check of the `word_count * 9` capacity bound computed in
        // `render_phrase`: 96 bytes of entropy renders as 72 words, and the
        // rendered text must never exceed 72 * 9 characters.
        let phrase = render_phrase(&[0xffu8; 96]).expect("renders");
        assert_eq!(phrase.word_count(), 72);
        assert!(
            phrase.expose().len() <= 72 * 9,
            "rendered text of length {} exceeds the 72 * 9 capacity bound",
            phrase.expose().len()
        );
    }

    #[test]
    fn phrase_debug_is_opaque() {
        let phrase = render_phrase(&[0u8; 16]).expect("renders");
        let dbg = format!("{:?}", phrase.0);
        assert!(!dbg.contains("abandon"), "leaked: {dbg}");
    }
}
