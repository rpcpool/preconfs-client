# preconfs-client

Client and protobuf definitions for the Triton Preconfs streams. Preconfs
are preconfirmed Solana transactions: the feeds deliver them while the slot
is still being built, before the transaction lands on chain.

| crate | what |
|---|---|
| `triton-preconfs-proto` | `proto/preconfs.proto` and the generated messages and gRPC clients |
| `triton-preconfs-client` | connection, feeds and regions, filters, transaction parsing |
| `examples/rust` | `preconfs-subscribe`, a CLI that subscribes and logs updates |

## Quick start

```rust
use triton_preconfs_client::{Connector, Feed, Filter, Filters, Region};

let client = Connector::new("https://preconfs.rpcpool.com")
    .x_token(token)
    .connect()
    .await?;
let region = Region::parse(Feed::Harmonic, "ams")?;
let filters = Filters::single(Filter::accounts([account]));
let mut stream = client.subscribe_harmonic(region, filters).await?;
while let Some(update) = stream.message().await? {
    println!("{update:?}");
}
```

The same shape works for the BAM feed with `Feed::Bam` and
`subscribe_bam`. The full program is in `examples/rust`:

```
cargo run -p preconfs-example -- --endpoint https://preconfs.rpcpool.com \
    --x-token $TOKEN --feed harmonic --region ams \
    --account TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA
```

## Connecting

- `preconfs.rpcpool.com` is anycast: the connection lands on the closest
  point of presence. `Connector::dial` pins one by address.
- Every request carries your `x-token`.
- A stream serves one feed in one region. Harmonic regions: ams, ewr, fra,
  lon, tyo, sgp, slc. BAM regions: `Feed::Bam.regions()` lists them.

## Filters

Filters are named; every matching update echoes the names that matched. A
transaction matches a filter when it satisfies every set condition:

- `account_include`: references any of these accounts
- `account_required`: references all of these accounts
- `signatures`: is one of these signatures
- `execution_results`: landed with one of these outcomes (Harmonic only)

Limits, checked client side before the request is sent: 64 filters per
stream, 10000 accounts per list, 1000 signatures per filter, 64 byte
names. Every filter must select something; full feed subscriptions are
refused.

## The stream

- Harmonic updates are framed per slot: `SlotStart`, the transactions,
  `SlotEnd`. After `SlotEnd` for a slot you hold everything your filters
  matched for it. A stream that subscribes while a slot is open joins at
  the next `SlotStart`. BAM has no framing; its updates carry the slot
  number per transaction.
- The server never drops matching transactions silently. If your account
  is over its coverage limit, withheld transactions are announced with a
  `CoverageClip` notice; if you cannot keep up, the stream is closed with
  an explicit error.
- Updates carry raw transaction bytes. `parse::parse_static_parts` extracts
  the signature and account keys without a full decode;
  `parse::parse_signature` is cheaper when only the signature is needed.

## Releases

Every release has an entry in [CHANGELOG.md](CHANGELOG.md). Released
clients keep working against newer servers.
