# preconfs-client

Client and protobuf definitions for the Triton Preconfs streams.

A preconfirmation is a transaction announced by the party building the
block the moment it is executed or committed into the slot, before any
shred exists and before any node reports it. It is the earliest signal that
a transaction is in a block, not the cluster's confirmation. Two feeds
produce them, Harmonic and BAM, each with its own regions.

Full documentation, including how each feed works and what the stream
guarantees, is at [docs.triton.one](https://docs.triton.one/chains/solana/preconfirmations-grpc).

| crate | what |
|---|---|
| [`triton-preconfs-proto`](https://crates.io/crates/triton-preconfs-proto) | `proto/preconfs.proto` and the generated messages and gRPC clients ([docs](https://docs.rs/triton-preconfs-proto)) |
| [`triton-preconfs-client`](https://crates.io/crates/triton-preconfs-client) | connection, feeds and regions, filters, transaction parsing ([docs](https://docs.rs/triton-preconfs-client)) |
| `examples/rust` | `preconfs-subscribe`, a CLI that subscribes and logs updates |

## Quick start

```rust
use triton_preconfs_client::{Connector, Event, Feed, Filter, Filters, Region};

let client = Connector::new("https://preconfs.rpcpool.com")
    .x_token(Some(token))
    .connect()
    .await?;
let region = Region::parse(Feed::Harmonic, "ams")?;
let filters = Filters::single(Filter::new().accounts([account]));
let mut stream = client.subscribe_harmonic(region, filters).await?;
while let Some(event) = stream.next().await {
    match event? {
        Event::Transaction(matched) => println!("{:?}", matched.transaction),
        Event::SlotEnd { slot } => println!("slot {slot} complete"),
        Event::Reconnected { .. } => println!("reconnected, data in between is lost"),
        _ => {}
    }
}
```

The same shape works for the BAM feed with `Feed::Bam` and
`subscribe_bam`. The full program is in `examples/rust`:

```
cargo run -p preconfs-example -- --endpoint https://preconfs.rpcpool.com \
    --x-token $TOKEN --region harmonic:ams \
    --account TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA
```

## Connecting

- `preconfs.rpcpool.com` is anycast: the connection lands on the closest
  point of presence, one of the servers behind that address.
  `Connector::dial` pins one by address.
- Every request carries your `x-token`, the token issued with your
  preconfs subscription. The server answers nothing without it, version
  and health checks included.
- A stream serves one feed in one region. Harmonic regions: ams, ewr, fra,
  lon, tyo, sgp, slc. BAM regions: `Feed::Bam.regions()` lists them.

## Filters

Filters are named; every matching update echoes the names that matched. A
transaction matches a filter when it satisfies every set condition:

- `account_include`: references any of these accounts
- `account_required`: references all of these accounts
- `account_exclude`: drops transactions referencing any of these; narrows a selection, cannot stand alone
- `signer_include`: signed by any of these accounts (fee payer included)
- `signer_exclude`: drops transactions signed by any of these; narrows a selection, cannot stand alone
- `instructions`: a top-level instruction invokes the program and its data
  passes every memcmp (bytes at an offset) and the exact data size, when
  set; any of the listed instruction filters. CPI instructions are not seen.
- `signatures`: is one of these signatures
- `execution_results`: landed with one of these outcomes (Harmonic only)

Account conditions see the static account keys only; an account a v0
transaction loads through a lookup table is not seen, by include or by
exclude.

Limits, checked client side before the request is sent: 64 filters per
stream, 10000 accounts per list, 1000 signatures per filter, 64 byte
names, 16 instruction filters per stream, 4 memcmps per instruction
filter, 128 bytes per memcmp, and a memcmp or data size must fit in 4096
bytes of instruction data. Every filter must select something; full feed
subscriptions are refused.

### Recipes

Transactions touching a pool, without a spammer that trades it all day:

```rust
Filter::new().accounts([pool]).exclude_accounts([spammer])
```

Transactions a wallet actually signed, as fee payer or cosigner. A wallet
that is only referenced, say as the recipient of a transfer, does not
match:

```rust
Filter::new().signers([wallet])
// only its trades on one program
Filter::new().signers([wallet]).accounts([program])
```

One instruction of a program, from any sender. Anchor programs start the
data with an 8 byte discriminator, the first 8 bytes of
`sha256("global:<instruction name>")`; Meteora DBC `create_config` is
`c9cff3724b6f2fbd`, so new configs arrive without their swaps:

```rust
let dbc: Pubkey = "dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN".parse()?;
Filter::new().instructions([
    InstructionFilter::new(dbc).memcmp(0, [201, 207, 243, 114, 75, 111, 47, 189]),
])
```

Native programs use a one byte tag, and `data_size` pins the layout. SPL
Token `TransferChecked` is tag 12 followed by an 8 byte amount and a 1 byte
decimals:

```rust
Filter::new().instructions([InstructionFilter::new(token_program).memcmp(0, [12]).data_size(10)])
```

A program invoked by the transaction, not merely mentioned in it:
`InstructionFilter::new(program)` matches a top-level instruction of that
program, while `accounts([program])` also matches transactions that only
pass the program id as an account.

Everything above except your own transactions:

```rust
Filter::new().instructions([swap]).exclude_signers([my_wallet])
```

The example CLI takes the same conditions: `--exclude`, `--signer`,
`--exclude-signer`, `--instruction PROGRAM` or `PROGRAM:OFFSET:HEX`, and
`--data-size`:

```
cargo run -p preconfs-example -- --x-token $TOKEN --region bam:fra \
    --instruction dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN:0:c9cff3724b6f2fbd
```

## The stream

- Harmonic events are framed per slot: `SlotStart`, the transactions,
  `SlotEnd`. After `SlotEnd` for a slot you hold everything your filters
  matched for it. A stream that subscribes while a slot is open joins at
  the next `SlotStart`. BAM has no framing; each transaction names its
  slot.
- The server never drops matching transactions silently. Withheld
  transactions are announced with an `Event::Clip` (see Coverage below);
  if you cannot keep up, the stream ends with an explicit error.
- Streams reconnect by default. When a point of presence restarts, the
  stream resubscribes with a backoff and yields `Event::Reconnected`; the
  data produced in between is gone. `Connector::reconnect` tunes the
  schedule, `Connector::no_reconnect` turns it off.
- Transactions carry raw bytes. `parse::parse_static_parts` returns the
  first signature and the static account keys; `parse::parse_signature`
  returns the signature alone.

## Coverage

Each account may receive up to a share of a feed's total traffic, measured
over a sliding window. Over that share, matching transactions are withheld
and the count is announced with `Event::Clip`. Staying over it ends the
stream with `ResourceExhausted`, and subscribing again is refused for a
cooloff period; with reconnect on, the stream retries by itself. Filters
that select only what you need keep you under the share.

## Releases

Every release has an entry in [CHANGELOG.md](CHANGELOG.md). Released
clients keep working against newer servers.
