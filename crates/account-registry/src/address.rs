//! CKB2021 full-format address: bech32m over
//! `0x00 ‖ code_hash ‖ hash_type ‖ args` with hrp `ckb` or `ckt` (RFC 0021).

use bech32::{Bech32m, Hrp};
use lantern_sdk_schema::{Network, ScriptTemplate};

use crate::error::RegistryError;

const FULL_FORMAT_TYPE: u8 = 0x00;

/// Encode a lock script as a CKB2021 full-format address.
pub fn encode_full(
    network: Network,
    template: &ScriptTemplate,
    args: &[u8],
) -> Result<String, RegistryError> {
    let mut payload = Vec::with_capacity(34 + args.len());
    payload.push(FULL_FORMAT_TYPE);
    payload.extend_from_slice(&template.code_hash);
    payload.push(template.hash_type);
    payload.extend_from_slice(args);
    let hrp = Hrp::parse(network.address_prefix()).map_err(|_| RegistryError::Address)?;
    bech32::encode::<Bech32m>(hrp, &payload).map_err(|_| RegistryError::Address)
}

#[cfg(test)]
mod tests {
    use lantern_sdk_schema::{Network, ScriptTemplate};

    use super::encode_full;

    fn secp_template() -> ScriptTemplate {
        let v = hex::decode("9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8")
            .expect("hex");
        let mut code_hash = [0u8; 32];
        code_hash.copy_from_slice(&v);
        ScriptTemplate {
            code_hash,
            hash_type: 0x01,
        }
    }

    #[test]
    fn rfc21_full_address_vector() {
        let args = hex::decode("b39bbc0b3673c7d36450bc14cfcdad2d559c6c64").expect("hex");
        assert_eq!(
            encode_full(Network::Mainnet, &secp_template(), &args).expect("encodes"),
            "ckb1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqdnnw7qkdnnclfkg59uzn8umtfd2kwxceqxwquc4"
        );
        assert_eq!(
            encode_full(Network::Testnet, &secp_template(), &args).expect("encodes"),
            "ckt1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqdnnw7qkdnnclfkg59uzn8umtfd2kwxceqgutnjd"
        );
    }

    #[test]
    fn lumos_first_receiving_address() {
        // blake160 of pub 0331b3c0… (lumos m/44'/309'/0'/0/0 from the BIP32 TV1 seed)
        let args = hex::decode("02e830bd6fe19912ffb7b0b134cbe53178b9e8f1").expect("hex");
        assert_eq!(
            encode_full(Network::Testnet, &secp_template(), &args).expect("encodes"),
            "ckt1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqgzaqct6mlpnyf0ldasky6vhef30zu73ugvy42t5"
        );
        assert_eq!(
            encode_full(Network::Mainnet, &secp_template(), &args).expect("encodes"),
            "ckb1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqgzaqct6mlpnyf0ldasky6vhef30zu73ugzk79pv"
        );
    }
}
