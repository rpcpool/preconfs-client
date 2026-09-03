# preconfs-client

Client and protobuf definitions for the Triton Preconfs streams. Preconfs
are preconfirmed Solana transactions: the feeds deliver them while the slot
is still being built, before the transaction lands on chain. There are two
feeds, each with its own regions: Harmonic, where a builder executes the
transaction and reports the outcome, and BAM, where the leader commits it
without reporting an outcome.

| crate | what |
|---|---|
| `triton-preconfs-proto` | `proto/preconfs.proto` and the generated messages and gRPC clients |
| `triton-preconfs-client` | connection, feeds and regions, filters, transaction parsing |
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
  preconfs subscription.
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
- Transactions carry raw bytes. `parse::parse_static_parts` extracts the
  signature and account keys without a full decode; `parse::parse_signature`
  is cheaper when only the signature is needed.

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
