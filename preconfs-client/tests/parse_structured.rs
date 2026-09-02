//! Structured mutations of valid transactions: every count field at its
//! boundaries, header values that disagree with the key count, lookup
//! table sections, and a measurement of how much the parser allocates for
//! a crafted length prefix before it looks at the payload.

use {
    solana_pubkey::Pubkey,
    std::{
        alloc::{GlobalAlloc, Layout, System},
        sync::atomic::{AtomicUsize, Ordering},
    },
    triton_preconfs_client::parse::{parse_signature, parse_static_parts},
};

struct Counting;

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static LARGEST: AtomicUsize = AtomicUsize::new(0);
/// The counters are process-wide, so the tests reading them run one at a time.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        LARGEST.fetch_max(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn compact_u16(value: u16) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rem = value;
    loop {
        let byte = (rem & 0x7f) as u8;
        rem >>= 7;
        if rem == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

struct Tx {
    num_signatures: u16,
    versioned: bool,
    header: [u8; 3],
    keys: Vec<Pubkey>,
    lookups: Vec<(u8, u8)>,
}

impl Tx {
    fn bytes(&self) -> Vec<u8> {
        let mut data = compact_u16(self.num_signatures);
        for i in 0..self.num_signatures {
            data.extend_from_slice(&[i as u8; 64]);
        }
        if self.versioned {
            data.push(0x80);
        }
        data.extend_from_slice(&self.header);
        data.extend(compact_u16(self.keys.len() as u16));
        for key in &self.keys {
            data.extend_from_slice(key.as_ref());
        }
        data.extend_from_slice(&[7u8; 32]);
        data.push(0);
        if self.versioned {
            data.extend(compact_u16(self.lookups.len() as u16));
            for (writable, readonly) in &self.lookups {
                data.extend_from_slice(&[0xABu8; 32]);
                data.push(*writable);
                data.extend(std::iter::repeat_n(0u8, usize::from(*writable)));
                data.push(*readonly);
                data.extend(std::iter::repeat_n(1u8, usize::from(*readonly)));
            }
        }
        data
    }
}

fn keys(n: usize) -> Vec<Pubkey> {
    (0..n)
        .map(|i| Pubkey::new_from_array([i as u8; 32]))
        .collect()
}

#[test]
fn boundary_counts_never_panic_and_agree_on_valid_input() {
    let _serial = SERIAL.lock().unwrap();
    for versioned in [false, true] {
        for num_signatures in [0u16, 1, 2, 127, 128, 255, 256] {
            for num_keys in [0usize, 1, 16, 17, 127, 128, 255, 256] {
                for header in [
                    [1, 0, 1],
                    [0, 0, 0],
                    [127, 255, 255],
                    [(num_keys + 1).min(127) as u8, 0, 0],
                ] {
                    let tx = Tx {
                        num_signatures,
                        versioned,
                        header,
                        keys: keys(num_keys),
                        lookups: if versioned {
                            vec![(0, 0), (255, 255)]
                        } else {
                            vec![]
                        },
                    };
                    let data = tx.bytes();
                    let parsed = parse_static_parts(&data);
                    let signature = parse_signature(&data);
                    if num_signatures == 0 {
                        assert!(parsed.is_err() && signature.is_err());
                        continue;
                    }
                    let (sig, parsed_keys) = parsed.unwrap_or_else(|e| panic!("valid layout parses: {e} (versioned={versioned} sigs={num_signatures} keys={num_keys} header={header:?})"));
                    assert_eq!(sig, signature.unwrap());
                    assert_eq!(sig.as_ref(), &[0u8; 64][..]);
                    assert_eq!(parsed_keys.len(), num_keys);
                    // Every single-byte mutation and every truncation of a
                    // valid transaction is at most an error.
                    for cut in 0..data.len() {
                        let _ = parse_static_parts(&data[..cut]);
                    }
                    for i in 0..data.len().min(600) {
                        let mut mutated = data.clone();
                        for value in [0x00, 0x7f, 0x80, 0xff] {
                            mutated[i] = value;
                            let _ = parse_static_parts(&mutated);
                            let _ = parse_signature(&mutated);
                        }
                    }
                }
            }
        }
    }
}

/// Measurement: the key count is trusted before the payload is checked, so
/// `[1, sig*64, header, 0xff 0xff 0x03]` (70 bytes) makes the parser
/// reserve 65535 * 32 bytes before it fails on the first missing key. Not a
/// panic; recorded as the largest single allocation for a 70-byte input.
#[test]
fn crafted_key_count_allocation_is_bounded_by_input_length() {
    let _serial = SERIAL.lock().unwrap();
    let mut data = vec![1u8];
    data.extend_from_slice(&[0u8; 64]);
    data.extend_from_slice(&[1, 0, 1, 0xff, 0xff, 0x03]);
    LARGEST.store(0, Ordering::SeqCst);
    let before = ALLOCATED.load(Ordering::SeqCst);
    assert!(parse_static_parts(&data).is_err());
    let allocated = ALLOCATED.load(Ordering::SeqCst) - before;
    let largest = LARGEST.load(Ordering::SeqCst);
    eprintln!(
        "{}-byte input: allocated {allocated} bytes, largest block {largest}",
        data.len()
    );
    assert!(
        largest <= data.len() * 32,
        "parser reserved {largest} bytes for a {}-byte input",
        data.len()
    );
}
