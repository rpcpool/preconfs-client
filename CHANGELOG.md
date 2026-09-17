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

### Fixed
- rustls 0.23.45 (RUSTSEC-2026-0285, TLS 1.3 handshake messages accepted
  across encryption level boundaries).
- The reconnect backoff no longer panics once the delay outgrows a
  `Duration` (attempt 69 with the defaults); the cap applies instead.
