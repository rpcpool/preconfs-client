//! Static transaction parsing must never panic on arbitrary bytes, and must
//! agree with the wire format on the cases the filters depend on.

use {
    solana_pubkey::Pubkey,
    triton_preconfs_client::parse::{parse_signature, parse_static_parts},
};

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

fn build_tx(versioned: bool, num_signatures: usize, keys: &[Pubkey]) -> Vec<u8> {
    let mut data = vec![num_signatures as u8];
    for i in 0..num_signatures {
        data.extend_from_slice(&[i as u8; 64]);
    }
    if versioned {
        data.push(0x80);
    }
    data.extend_from_slice(&[num_signatures as u8, 0, 1]);
    data.push(keys.len() as u8);
    for key in keys {
        data.extend_from_slice(key.as_ref());
    }
    data.extend_from_slice(&[7u8; 32]);
    data.push(0);
    data
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..200_000 {
        let len = (rng.next() % 300) as usize;
        let data: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        let _ = parse_static_parts(&data);
        let _ = parse_signature(&data);
    }
}

#[test]
fn every_truncation_of_a_valid_tx_is_an_error_not_a_panic() {
    let keys: Vec<Pubkey> = (0..40u8).map(|i| Pubkey::new_from_array([i; 32])).collect();
    for versioned in [false, true] {
        let data = build_tx(versioned, 3, &keys);
        assert!(parse_static_parts(&data).is_ok());
        for cut in 0..data.len() {
            // Anything cut before the last key must be Truncated; cutting
            // after the keys still parses (the parser stops at the keys).
            let result = parse_static_parts(&data[..cut]);
            let keys_end = 1 + 3 * 64 + usize::from(versioned) + 3 + 1 + 40 * 32;
            if cut < keys_end {
                assert!(result.is_err(), "cut at {cut} should be an error");
            } else {
                assert!(result.is_ok(), "cut at {cut} should parse");
            }
        }
    }
}

#[test]
fn multi_signature_transactions_skip_all_signatures() {
    let keys = [Pubkey::new_from_array([9u8; 32])];
    let data = build_tx(true, 3, &keys);
    let (sig, parsed) = parse_static_parts(&data).expect("parses");
    assert_eq!(sig.as_ref(), &[0u8; 64][..]);
    assert_eq!(parsed.as_slice(), &keys);
}

/// Huge declared counts (compact-u16 max is 65535) must fail cleanly.
#[test]
fn absurd_counts_are_truncated_errors() {
    // 65535 signatures declared: [0xff, 0xff, 0x03].
    let data = [0xffu8, 0xff, 0x03, 0, 0, 0];
    assert!(parse_static_parts(&data).is_err());
    assert!(parse_signature(&data).is_err());
    // One signature, then 65535 keys declared.
    let mut data = vec![1u8];
    data.extend_from_slice(&[0u8; 64]);
    data.extend_from_slice(&[1, 0, 1, 0xff, 0xff, 0x03]);
    assert!(parse_static_parts(&data).is_err());
}

/// Lookup-table addresses are not in the static keys; documented, and the
/// filters only look at static keys, so this pins the contract.
#[test]
fn v0_lookup_table_keys_are_not_part_of_account_keys() {
    let keys = [Pubkey::new_from_array([1u8; 32])];
    let mut data = build_tx(true, 1, &keys);
    // One address-table lookup after the (empty) instructions.
    data.push(1);
    data.extend_from_slice(&[0xABu8; 32]); // table account
    data.push(1);
    data.push(0); // writable index
    data.push(0); // no readonly indexes
    let (_, parsed) = parse_static_parts(&data).expect("parses");
    assert_eq!(parsed.as_slice(), &keys);
}
