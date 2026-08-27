# preconfs-client

Client and protobuf definitions for the Triton Preconfs streams
(`preconfs.Harmonic`, `preconfs.BAM`).

| crate | what |
|---|---|
| `triton-preconfs-proto` | `proto/preconfs.proto` and the generated messages and gRPC clients |
| `triton-preconfs-client` | connection, feeds and regions, filters, transaction parsing |
| `examples/rust` | `preconfs-subscribe`, a CLI that subscribes and logs updates |

```
cargo run -p preconfs-example -- --endpoint https://preconfs.rpcpool.com \
    --x-token $TOKEN --feed harmonic --region ams \
    --account TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA
```

## Versioning

- Each crate is versioned and tagged on its own: `proto-vX.Y.Z`,
  `client-vX.Y.Z`. The crate version in `Cargo.toml` must match the tag.
- Any change to `preconfs.proto` bumps the proto crate and the client crate
  that depends on it.
- SemVer. Before 1.0 a minor bump may change the API, a patch bump never
  does. Wire compatibility with the server is stated per release in the
  changelog.
- Every release has an entry in `CHANGELOG.md`; the `Unreleased` section is
  moved under the version at release time.
