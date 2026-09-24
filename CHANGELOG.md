# Changelog

All notable changes to the crates in this repository.

- Each crate is versioned and tagged on its own: `proto-vX.Y.Z`,
  `client-vX.Y.Z`. The crate version in `Cargo.toml` must match the tag.
- Any change to `preconfs.proto` bumps the proto crate and the client crate
  that depends on it.
- Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), versions
  follow [SemVer](https://semver.org/). Until 1.0, a minor bump may break
  the API; a patch bump never does.
- The `Unreleased` section is moved under the version at release time.

## Unreleased

### Added
- `account_exclude` on `TransactionFilter` (field 2) and `Filter::exclude_accounts`:
  drops transactions that reference any of the listed accounts. It narrows
  a positive filter; a filter with only exclusions is refused.
  `TransactionFilter` gains a field, so struct literals need
  `..Default::default()`.
- `signer_include` and `signer_exclude` on `TransactionFilter` (fields 6
  and 7), `Filter::signers` and `Filter::exclude_signers`: match or drop
  transactions by the accounts that signed them. Signer exclusions alone
  are refused like account exclusions.
- `instructions` on `TransactionFilter` (field 8), `InstructionFilter` and
  `Memcmp`, `Filter::instructions`: match transactions by a top-level
  instruction's program, bytes at data offsets and exact data size, the
  getProgramAccounts filters applied to instruction data. New limits:
  `MAX_INSTRUCTION_FILTERS` per stream, `MAX_MEMCMPS_PER_INSTRUCTION`,
  `MAX_MEMCMP_BYTES`, `MAX_INSTRUCTION_DATA_BYTES`, and new
  `FilterError` variants for each.

## client-v0.2.0 - 2026-09-22

### Fixed
- parse: v1 transactions (SIMD-0385) parse. Their signatures follow the
  message instead of leading it, and the parser read them at the legacy
  offset, so they came out as malformed. Parsing now goes through the
  Solana sdk, which covers legacy, v0 and v1 alike.

### Changed
- The error enums and `Event` are `#[non_exhaustive]`: a new variant is no
  longer a breaking change. Matches outside the crate need a wildcard arm.
- `Filter` and `Reconnect` are `#[non_exhaustive]` and built through their
  methods; `Reconnect` gains `initial_interval`, `multiplier`,
  `max_interval` and `max_retries`. Struct literals outside the crate no
  longer compile; the fields stay readable.
- `stream::FeedUpdate` is sealed.
- `parse::parse` returns the decoded `VersionedTransaction`;
  `parse_static_parts` returns the static keys as a `Vec<Pubkey>` and the
  `AccountKeys` alias is gone. `parse::ParseError` is
  `Malformed(wincode::ReadError)` or `NoSignature`; `Truncated`, `BadLength`
  and `UnsupportedVersion` are gone. The bytes must be one whole
  transaction: trailing bytes are an error.

## proto-v0.1.0, client-v0.1.0 - 2026-09-17

First release of both crates.

### Fixed
- proto comments: a half sentence left from the removed BAM slot boundaries,
  and a merged line in the stream contract. No wire change.

### Added
- `triton-preconfs-proto`: the `preconfs.proto` definitions with the
  generated tonic clients for the Harmonic and BAM services.
- `triton-preconfs-client`: `Connector` (anycast or pinned dial, `x-token`
  on every request, keepalive and optional compression), typed
  `HarmonicStream` and `BamStream` with reconnect, `Filters` validated
  against the server limits, transaction parsing for legacy, v0 and v1
  messages, per domain errors.

### Changed
- Minimum supported Rust is 1.89, what the dependencies need.

### Fixed
- `Connector::health` sends the `x-token` like every other call; the server
  refuses health checks without one since server 0.2.0.
- The reconnect backoff no longer panics once the delay outgrows a
  `Duration` (attempt 69 with the defaults); the cap applies instead.
- rustls 0.23.45 (RUSTSEC-2026-0285, TLS 1.3 handshake messages accepted
  across encryption level boundaries).
