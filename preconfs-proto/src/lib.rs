//! Triton Preconfs API: the messages and gRPC clients generated from
//! `proto/preconfs.proto` (services `preconfs.Harmonic` and `preconfs.BAM`).

pub mod preconfs {
    // Generated code does not follow the workspace lints.
    #![allow(clippy::clone_on_ref_ptr, clippy::missing_const_for_fn)]
    tonic::include_proto!("preconfs");
}

/// The schema this crate was built from, for tooling in other languages.
pub const PROTO_SOURCE: &str = include_str!("../proto/preconfs.proto");

pub use {prost, tonic};
