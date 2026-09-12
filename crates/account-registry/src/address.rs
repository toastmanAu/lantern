//! CKB2021 full-format address: bech32m over
//! `0x00 ‖ code_hash ‖ hash_type ‖ args` with hrp `ckb` or `ckt` (RFC 0021).

use bech32::primitives::decode::CheckedHrpstring;
use bech32::{Bech32m, Hrp};
use lantern_sdk_schema::{Network, ScriptTemplate};

use crate::error::RegistryError;

const FULL_FORMAT_TYPE: u8 = 0x00;

/// `0x00` format byte, 32-byte code hash, one hash-type byte. Args follow.
const HEADER_LEN: usize = 1 + 32 + 1;

/// The `ScriptHashType` discriminants CKB defines.
///
/// Not a dense range: `Data = 0, Type = 1, Data1 = 2, Data2 = 4`, because the
/// low bit encodes data-vs-type and the high bits the VM version. There is no
/// hash type 3, and a script carrying one is unspendable — so an address
/// claiming one is refused here rather than rendered as a payable destination.
const VALID_HASH_TYPES: [u8; 4] = [0x00, 0x01, 0x02, 0x04];

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

/// Decode a CKB2021 full-format address into the script it names.
///
/// The inverse of [`encode_full`], sharing its constants rather than
/// restating the payload layout. Returns the template and the args, which is
/// the same pair the encoder takes — converting those into whichever `Script`
/// type a caller needs is the caller's business, and is what keeps this crate
/// free of chain types.
///
/// Refuses anything that is not a full-format address **on `network`**.
/// Nothing downstream would catch a mismatch: a mainnet lock script is a
/// perfectly well-formed script on testnet, so a mis-prefixed recipient is a
/// real payment to an address whose owner is on another chain.
///
/// # Errors
///
/// Returns [`RegistryError::InvalidAddress`] if the string is not bech32m, is
/// prefixed for another network, is not the full (`0x00`) format, is too
/// short to carry a code hash and hash type, or names a hash type CKB does
/// not define.
pub fn decode_full(
    network: Network,
    address: &str,
) -> Result<(ScriptTemplate, Vec<u8>), RegistryError> {
    // `Bech32m` explicitly, not `bech32::decode`, which accepts either
    // checksum. A bech32 (not -m) checksum is what the deprecated pre-2021
    // formats carry, and those payloads mean something else entirely.
    let checked = CheckedHrpstring::new::<Bech32m>(address)
        .map_err(|_| RegistryError::InvalidAddress("not a bech32m string"))?;
    if !checked
        .hrp()
        .as_str()
        .eq_ignore_ascii_case(network.address_prefix())
    {
        return Err(RegistryError::InvalidAddress(
            "address belongs to a different CKB network",
        ));
    }

    let payload: Vec<u8> = checked.byte_iter().collect();
    if payload.len() < HEADER_LEN {
        return Err(RegistryError::InvalidAddress(
            "too short to carry a code hash and hash type",
        ));
    }
    if payload[0] != FULL_FORMAT_TYPE {
        return Err(RegistryError::InvalidAddress(
            "not a CKB2021 full-format address",
        ));
    }

    let mut code_hash = [0u8; 32];
    code_hash.copy_from_slice(&payload[1..33]);
    let hash_type = payload[33];
    if !VALID_HASH_TYPES.contains(&hash_type) {
        return Err(RegistryError::InvalidAddress(
            "names a script hash type CKB does not define",
        ));
    }

    Ok((
        ScriptTemplate {
            code_hash,
            hash_type,
        },
        payload[HEADER_LEN..].to_vec(),
    ))
}

#[cfg(test)]
mod tests {
    use lantern_sdk_schema::{Network, ScriptTemplate};

    use super::{decode_full, encode_full};
    use crate::error::RegistryError;

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

    #[test]
    fn the_rfc21_vector_decodes_back_to_the_script_it_encodes() {
        // The same published vector the encoder is pinned against, read the
        // other way. A decoder checked only against this crate's own encoder
        // would agree with it even if both were wrong about the payload
        // layout; here the string itself is the oracle.
        let args = hex::decode("b39bbc0b3673c7d36450bc14cfcdad2d559c6c64").expect("hex");
        let (template, decoded) = decode_full(
            Network::Mainnet,
            "ckb1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqdnnw7qkdnnclfkg59uzn8umtfd2kwxceqxwquc4",
        )
        .expect("decodes");
        assert_eq!(template, secp_template());
        assert_eq!(decoded, args);
    }

    #[test]
    fn every_encoding_round_trips_for_both_networks_and_odd_arg_lengths() {
        // 0 and 32 bytes as well as 20: a PQ lock's args are 32 bytes, and an
        // args-less lock is legal. A decoder that assumed 20 would pass the
        // secp vector and silently truncate the rest.
        for network in [Network::Mainnet, Network::Testnet] {
            for len in [0usize, 1, 20, 32] {
                let args: Vec<u8> = (0..len).map(|i| u8::try_from(i).expect("small")).collect();
                let text = encode_full(network, &secp_template(), &args).expect("encodes");
                assert_eq!(
                    decode_full(network, &text).expect("decodes"),
                    (secp_template(), args),
                    "{network:?} with {len}-byte args"
                );
            }
        }
    }

    #[test]
    fn an_address_for_the_other_chain_is_refused() {
        // Nothing downstream would notice: a mainnet lock script is a
        // perfectly well-formed script on testnet, so a mis-prefixed
        // recipient is a payment to a real address on the wrong chain.
        let args = hex::decode("b39bbc0b3673c7d36450bc14cfcdad2d559c6c64").expect("hex");
        let mainnet = encode_full(Network::Mainnet, &secp_template(), &args).expect("encodes");
        assert!(matches!(
            decode_full(Network::Testnet, &mainnet),
            Err(RegistryError::InvalidAddress(_))
        ));
    }

    #[test]
    fn short_format_and_malformed_addresses_are_refused() {
        let template = secp_template();
        // A deprecated 2019 short-format payload: type byte 0x01 rather than
        // 0x00, and a code-hash *index* rather than the hash itself. Its
        // bytes decode as bech32 but mean something entirely different.
        let short = {
            let mut payload = vec![0x01u8, 0x00];
            payload.extend_from_slice(
                &hex::decode("b39bbc0b3673c7d36450bc14cfcdad2d559c6c64").expect("hex"),
            );
            bech32::encode::<bech32::Bech32>(bech32::Hrp::parse("ckb").expect("hrp"), &payload)
                .expect("encodes")
        };
        assert!(matches!(
            decode_full(Network::Mainnet, &short),
            Err(RegistryError::InvalidAddress(_))
        ));

        // Truncated: a format byte and a partial code hash, no hash type.
        let truncated = bech32::encode::<bech32::Bech32m>(
            bech32::Hrp::parse("ckb").expect("hrp"),
            &[0x00u8; 20],
        )
        .expect("encodes");
        assert!(matches!(
            decode_full(Network::Mainnet, &truncated),
            Err(RegistryError::InvalidAddress(_))
        ));

        // Not bech32 at all.
        assert!(matches!(
            decode_full(Network::Mainnet, "definitely not an address"),
            Err(RegistryError::InvalidAddress(_))
        ));

        // A hash type the CKB VM does not define. `ScriptHashType` is
        // `Data=0, Type=1, Data1=2, Data2=4` — there is no 3, and a script
        // carrying one is unspendable.
        let bad_hash_type = {
            let mut payload = vec![0x00u8];
            payload.extend_from_slice(&template.code_hash);
            payload.push(0x03);
            bech32::encode::<bech32::Bech32m>(bech32::Hrp::parse("ckb").expect("hrp"), &payload)
                .expect("encodes")
        };
        assert!(matches!(
            decode_full(Network::Mainnet, &bad_hash_type),
            Err(RegistryError::InvalidAddress(_))
        ));
    }

    #[test]
    fn a_bech32_checksum_is_refused_where_bech32m_is_required() {
        // RFC 0021 full addresses are bech32m. `bech32::decode` accepts
        // either variant, so a decoder built on it would accept a string
        // whose checksum says it is a pre-2021 address.
        let mut payload = vec![0x00u8];
        payload.extend_from_slice(&secp_template().code_hash);
        payload.push(0x01);
        payload.extend_from_slice(&[0xab; 20]);
        let wrong_checksum =
            bech32::encode::<bech32::Bech32>(bech32::Hrp::parse("ckb").expect("hrp"), &payload)
                .expect("encodes");
        assert!(matches!(
            decode_full(Network::Mainnet, &wrong_checksum),
            Err(RegistryError::InvalidAddress(_))
        ));
    }
}
