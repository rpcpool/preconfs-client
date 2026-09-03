//! Minimal transaction-byte parsing: enough to filter and match on raw
//! transactions without a full decode.

use {smallvec::SmallVec, solana_pubkey::Pubkey, solana_signature::Signature, thiserror::Error};

/// Static account keys of a transaction. Inline capacity covers typical
/// transactions so parsing does not heap-allocate per transaction.
pub type AccountKeys = SmallVec<[Pubkey; 16]>;

/// Bytes that are not a transaction the parser understands.
#[derive(Debug, Error)]
pub enum ParseError {
    /// The data ends before the field being read.
    #[error("truncated transaction")]
    Truncated,
    /// A compact-u16 length prefix that is not valid.
    #[error("invalid compact-u16 length prefix")]
    BadLength,
    /// A transaction with zero signatures.
    #[error("transaction has no signatures")]
    NoSignature,
    /// A message version other than legacy, v0 or v1. Keys read from the
    /// wrong offsets would match nothing or the wrong filters, so this is
    /// an error, not a guess.
    #[error("unsupported message version {0}")]
    UnsupportedVersion(u8),
}

/// Versioned messages start with this bit set; the low bits are the version.
const VERSION_PREFIX: u8 = 0x80;
/// SIMD-0385: legacy header, u32 config mask, 32-byte lifetime specifier,
/// u8 instruction count, u8 address count, then the addresses.
const V1_FIXED_HEADER: usize = 3 + 4 + 32 + 1;

/// Reads a Solana compact-u16 ("short vec") length prefix.
fn read_compact_u16(data: &[u8], pos: &mut usize) -> Result<usize, ParseError> {
    let mut value = 0usize;
    for i in 0..3 {
        let byte = *data.get(*pos).ok_or(ParseError::Truncated)? as usize;
        *pos += 1;
        value |= (byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            // The third byte may only contribute the two remaining bits of a u16.
            if i == 2 && byte > 0x03 {
                return Err(ParseError::BadLength);
            }
            return Ok(value);
        }
    }
    Err(ParseError::BadLength)
}

/// Extracts only the first signature from raw transaction bytes. Cheaper than
/// `parse_static_parts` when account keys are not needed (monitoring paths).
pub fn parse_signature(data: &[u8]) -> Result<Signature, ParseError> {
    let mut pos = 0;
    let num_signatures = read_compact_u16(data, &mut pos)?;
    if num_signatures == 0 {
        return Err(ParseError::NoSignature);
    }
    let signature: [u8; 64] = data
        .get(pos..pos + 64)
        .ok_or(ParseError::Truncated)?
        .try_into()
        .expect("slice length checked");
    Ok(Signature::from(signature))
}

/// Extracts the first signature and the static account keys from raw
/// transaction bytes (legacy, v0 and v1 message formats), without a full
/// decode. An unknown message version is an error, not a guess: keys read
/// from the wrong offsets would match nothing or the wrong filters.
pub fn parse_static_parts(data: &[u8]) -> Result<(Signature, AccountKeys), ParseError> {
    let mut pos = 0;
    let num_signatures = read_compact_u16(data, &mut pos)?;
    if num_signatures == 0 {
        return Err(ParseError::NoSignature);
    }
    let signature: [u8; 64] = data
        .get(pos..pos + 64)
        .ok_or(ParseError::Truncated)?
        .try_into()
        .expect("slice length checked");
    pos += num_signatures * 64;

    // Versioned messages carry a version prefix byte with the high bit set;
    // legacy messages start directly with the header.
    let first = *data.get(pos).ok_or(ParseError::Truncated)?;
    let num_keys = if first & VERSION_PREFIX == 0 {
        // Legacy: 3-byte header, then a compact-u16 key count.
        pos += 3;
        read_compact_u16(data, &mut pos)?
    } else {
        match first & !VERSION_PREFIX {
            0 => {
                pos += 1 + 3;
                read_compact_u16(data, &mut pos)?
            }
            1 => {
                pos += 1 + V1_FIXED_HEADER;
                let count = *data.get(pos).ok_or(ParseError::Truncated)?;
                pos += 1;
                usize::from(count)
            }
            version => return Err(ParseError::UnsupportedVersion(version)),
        }
    };
    // The declared count is upstream data: a crafted 65535 must not reserve
    // 2 MB per message; the payload cannot hold more keys than it has bytes.
    let mut account_keys =
        AccountKeys::with_capacity(num_keys.min(data.len().saturating_sub(pos) / 32));
    for _ in 0..num_keys {
        let key: [u8; 32] = data
            .get(pos..pos + 32)
            .ok_or(ParseError::Truncated)?
            .try_into()
            .expect("slice length checked");
        account_keys.push(Pubkey::new_from_array(key));
        pos += 32;
    }
    Ok((Signature::from(signature), account_keys))
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        solana_hash::Hash,
        solana_message::{MessageHeader, v1},
    };

    /// Real v1 bytes from the solana-message serializer, wrapped in the
    /// transaction envelope (one signature).
    fn build_v1_tx(signature: [u8; 64], keys: &[Pubkey]) -> Vec<u8> {
        let message = v1::Message::new(
            MessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 1,
            },
            v1::TransactionConfig::empty()
                .with_compute_unit_limit(200_000)
                .with_priority_fee(5),
            Hash::new_from_array([7u8; 32]),
            keys.to_vec(),
            vec![],
        );
        let mut data = vec![1u8];
        data.extend_from_slice(&signature);
        data.extend_from_slice(&message.serialize());
        data
    }

    #[test]
    fn parses_v1() {
        let signature = [42u8; 64];
        let keys = [
            Pubkey::new_from_array([1u8; 32]),
            Pubkey::new_from_array([2u8; 32]),
            Pubkey::new_from_array([3u8; 32]),
        ];
        let data = build_v1_tx(signature, &keys);
        assert_eq!(data[65], 0x81, "v1 prefix");
        let (parsed_signature, parsed_keys) = parse_static_parts(&data).unwrap();
        assert_eq!(parsed_signature, Signature::from(signature));
        assert_eq!(parsed_keys.as_slice(), &keys);
        assert_eq!(parse_signature(&data).unwrap(), Signature::from(signature));
    }

    #[test]
    fn unknown_message_version_is_rejected() {
        let mut data = vec![1u8];
        data.extend_from_slice(&[0u8; 64]);
        data.push(0x82);
        data.extend_from_slice(&[0u8; 64]);
        assert!(matches!(
            parse_static_parts(&data),
            Err(ParseError::UnsupportedVersion(2))
        ));
    }

    fn build_tx(versioned: bool, signature: [u8; 64], keys: &[Pubkey]) -> Vec<u8> {
        let mut data = vec![1u8]; // one signature
        data.extend_from_slice(&signature);
        if versioned {
            data.push(0x80); // v0 prefix
        }
        data.extend_from_slice(&[1, 0, 1]); // header
        data.push(keys.len() as u8);
        for key in keys {
            data.extend_from_slice(key.as_ref());
        }
        data.extend_from_slice(&[7u8; 32]); // recent blockhash
        data.push(0); // no instructions
        data
    }

    #[test]
    fn parses_legacy_and_v0() {
        let signature = [42u8; 64];
        let keys = [
            Pubkey::new_from_array([1u8; 32]),
            Pubkey::new_from_array([2u8; 32]),
            Pubkey::new_from_array([3u8; 32]),
        ];
        for versioned in [false, true] {
            let data = build_tx(versioned, signature, &keys);
            let (sig, parsed) = parse_static_parts(&data).expect("parses");
            assert_eq!(sig, Signature::from(signature));
            assert_eq!(parsed.as_slice(), keys);
        }
    }

    #[test]
    fn rejects_truncated() {
        let data = build_tx(false, [42u8; 64], &[Pubkey::new_from_array([1u8; 32])]);
        assert!(parse_static_parts(&data[..40]).is_err());
        assert!(parse_static_parts(&[]).is_err());
        assert!(parse_static_parts(&[0]).is_err()); // zero signatures
    }

    #[test]
    fn compact_u16_two_bytes() {
        // 300 = 0b1_0010_1100 -> [0xac, 0x02]
        let mut data = vec![0xacu8, 0x02];
        let mut pos = 0;
        assert_eq!(read_compact_u16(&data, &mut pos).unwrap(), 300);
        assert_eq!(pos, 2);
        // Overflowing third byte is rejected.
        data = vec![0xff, 0xff, 0x04];
        pos = 0;
        assert!(read_compact_u16(&data, &mut pos).is_err());
    }
}
