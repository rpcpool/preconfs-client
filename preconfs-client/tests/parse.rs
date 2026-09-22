//! The parser against serialized transactions of every message version, plus
//! the malformed inputs it must refuse without a panic.

use {
    solana_hash::Hash,
    solana_message::{
        Message as LegacyMessage, MessageHeader, VersionedMessage,
        compiled_instruction::CompiledInstruction, v0, v1,
    },
    solana_pubkey::Pubkey,
    solana_signature::Signature,
    solana_transaction::versioned::VersionedTransaction,
    triton_preconfs_client::parse::{ParseError, parse, parse_signature, parse_static_parts},
};

const fn header(num_signatures: usize) -> MessageHeader {
    MessageHeader {
        num_required_signatures: num_signatures as u8,
        num_readonly_signed_accounts: 0,
        num_readonly_unsigned_accounts: 1,
    }
}

fn signatures(count: usize) -> Vec<Signature> {
    (0..count)
        .map(|i| Signature::from([i as u8 + 40; 64]))
        .collect()
}

fn keys(count: usize) -> Vec<Pubkey> {
    (0..count)
        .map(|i| Pubkey::new_from_array([i as u8 + 1; 32]))
        .collect()
}

fn instructions() -> Vec<CompiledInstruction> {
    vec![
        CompiledInstruction::new_from_raw_parts(0, vec![1, 2, 3], vec![0]),
        CompiledInstruction::new_from_raw_parts(0, vec![9; 300], vec![]),
    ]
}

/// The wire bytes of a transaction: the signature list in front for legacy
/// and v0, the message first and the signatures last for v1.
fn serialize(message: VersionedMessage, num_signatures: usize) -> Vec<u8> {
    wincode::serialize(&VersionedTransaction {
        signatures: signatures(num_signatures),
        message,
    })
    .expect("transactions serialize")
}

fn legacy(num_signatures: usize, keys: &[Pubkey]) -> Vec<u8> {
    serialize(
        VersionedMessage::Legacy(LegacyMessage {
            header: header(num_signatures),
            account_keys: keys.to_vec(),
            recent_blockhash: Hash::new_from_array([7u8; 32]),
            instructions: instructions(),
        }),
        num_signatures,
    )
}

fn v0(num_signatures: usize, keys: &[Pubkey]) -> Vec<u8> {
    serialize(
        VersionedMessage::V0(v0::Message {
            header: header(num_signatures),
            account_keys: keys.to_vec(),
            recent_blockhash: Hash::new_from_array([7u8; 32]),
            instructions: instructions(),
            address_table_lookups: vec![v0::MessageAddressTableLookup {
                account_key: Pubkey::new_from_array([0xAB; 32]),
                writable_indexes: vec![0, 1],
                readonly_indexes: vec![2],
            }],
        }),
        num_signatures,
    )
}

fn v1(num_signatures: usize, keys: &[Pubkey]) -> Vec<u8> {
    serialize(
        VersionedMessage::V1(v1::Message::new(
            header(num_signatures),
            v1::TransactionConfig::empty()
                .with_compute_unit_limit(200_000)
                .with_priority_fee(5)
                .with_heap_size(64 * 1024),
            Hash::new_from_array([7u8; 32]),
            keys.to_vec(),
            instructions(),
        )),
        num_signatures,
    )
}

fn all_formats(num_signatures: usize, keys: &[Pubkey]) -> [(&'static str, Vec<u8>); 3] {
    [
        ("legacy", legacy(num_signatures, keys)),
        ("v0", v0(num_signatures, keys)),
        ("v1", v1(num_signatures, keys)),
    ]
}

/// Deterministic xorshift so the test needs no rand dependency.
struct Rng(u64);

impl Rng {
    const fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

#[test]
fn every_format_yields_the_first_signature_and_the_static_keys() {
    for num_signatures in [1usize, 2, 12, 64] {
        for num_keys in [1usize, 3, 16, 17, 64, 255] {
            let keys = keys(num_keys);
            for (name, data) in all_formats(num_signatures, &keys) {
                let case = format!("{name} sigs={num_signatures} keys={num_keys}");
                let (signature, parsed_keys) = parse_static_parts(&data).expect(&case);
                assert_eq!(signature, signatures(1)[0], "{case}");
                assert_eq!(parsed_keys, keys, "{case}: static keys only");
                assert_eq!(parse_signature(&data).expect(&case), signature, "{case}");
                let transaction = parse(&data).expect(&case);
                assert_eq!(transaction.signatures, signatures(num_signatures), "{case}");
                assert_eq!(transaction.message.static_account_keys(), keys, "{case}");
            }
        }
    }
}

/// Pins the v1 framing: the version byte comes first and the signatures are
/// the last bytes.
#[test]
fn v1_carries_the_message_first_and_the_signatures_last() {
    let data = v1(2, &keys(3));
    assert_eq!(data[0], 0x81);
    let tail = &data[data.len() - 128..];
    assert_eq!(&tail[..64], signatures(2)[0].as_ref());
    assert_eq!(&tail[64..], signatures(2)[1].as_ref());
}

/// A transaction without signatures decodes, since that is what the bytes
/// say, but has no first signature to hand out.
#[test]
fn zero_signatures_is_an_error() {
    for (name, data) in all_formats(0, &keys(3)) {
        assert!(parse(&data).expect(name).signatures.is_empty(), "{name}");
        assert!(
            matches!(parse_static_parts(&data), Err(ParseError::NoSignature)),
            "{name}"
        );
        assert!(
            matches!(parse_signature(&data), Err(ParseError::NoSignature)),
            "{name}"
        );
    }
}

/// The bytes are one whole transaction: anything shorter or longer is not
/// the transaction the builder executed.
#[test]
fn truncations_and_trailing_bytes_are_errors() {
    for (name, data) in all_formats(2, &keys(3)) {
        for cut in 0..data.len() {
            assert!(
                parse_static_parts(&data[..cut]).is_err(),
                "{name} cut {cut}"
            );
            assert!(parse_signature(&data[..cut]).is_err(), "{name} cut {cut}");
        }
        let mut trailing = data.clone();
        trailing.push(0);
        assert!(
            matches!(parse_static_parts(&trailing), Err(ParseError::Malformed(_))),
            "{name} trailing byte"
        );
    }
}

#[test]
fn unknown_versions_are_errors() {
    // A version byte that is not v1 where a transaction starts.
    for first in [0x80u8, 0x82, 0xff] {
        let mut data = vec![first, 1, 0, 1];
        data.extend_from_slice(&[0u8; 200]);
        assert!(matches!(
            parse_static_parts(&data),
            Err(ParseError::Malformed(_))
        ));
    }
    // A v1 message behind a legacy signature list, a framing that does not
    // exist on the wire.
    let transaction = v1(1, &keys(3));
    let message = &transaction[..transaction.len() - 64];
    let mut data = vec![1u8];
    data.extend_from_slice(&[0u8; 64]);
    data.extend_from_slice(message);
    assert!(matches!(
        parse_static_parts(&data),
        Err(ParseError::Malformed(_))
    ));
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..200_000 {
        let len = (rng.next() % 300) as usize;
        let mut data: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        let _ = parse_static_parts(&data);
        let _ = parse_signature(&data);
        // Random first bytes take the v1 path once in 256 tries; force it.
        if let Some(first) = data.first_mut() {
            *first = 0x81;
            let _ = parse_static_parts(&data);
            let _ = parse_signature(&data);
        }
    }
}

/// Every truncation and every single-byte mutation of a real transaction is
/// at most an error.
#[test]
fn mutations_of_valid_transactions_never_panic() {
    for (_, data) in all_formats(2, &keys(17)) {
        for i in 0..data.len() {
            let mut mutated = data.clone();
            for value in [0x00, 0x7f, 0x80, 0x81, 0xff] {
                mutated[i] = value;
                let _ = parse_static_parts(&mutated);
                let _ = parse_signature(&mutated);
            }
        }
    }
}
