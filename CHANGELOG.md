# Changelog

All notable changes to the crates in this repository. Each crate has its own
version and tag; a proto change bumps both crates.

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), versions
follow [SemVer](https://semver.org/). Until 1.0, a minor bump may break the
API; a patch bump never does.

## Unreleased

### triton-preconfs-proto 0.1.0

- `preconfs.proto`: services `Harmonic` and `BAM`, `SubscribeRequest` with
  named `TransactionFilter`s and a required feed-typed region, updates with
  slot start/end, transactions, ping and `CoverageClip`.

### triton-preconfs-client 0.1.0

- `Connector`: https with native roots, `x-token`, keepalive, adaptive
  window, `dial` to pin one point of presence behind the anycast address.
- `Feed` and `Region` with the served region names.
- `Filters` validated against the server limits before the request is sent.
- `parse`: first signature and static account keys from raw transaction
  bytes (legacy, v0 and v1 formats).
