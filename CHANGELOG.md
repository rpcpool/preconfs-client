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
- `account_exclude` on `TransactionFilter` (field 2) and `Filter::exclude`:
  drop transactions referencing any of the listed accounts. A further
  condition on a positive filter; a filter with only exclusions is refused.

### Fixed
- proto comments: a half sentence left from the removed BAM slot boundaries,
  and a merged line in the stream contract. No wire change.

## proto-v0.1.0, client-v0.1.0 - 2026-09-17

First release of both crates.

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
