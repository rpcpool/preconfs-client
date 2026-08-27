//! Client for the Triton Preconfs streams (`preconfs.Harmonic` and
//! `preconfs.BAM`): connection with TLS and the `x-token`, feed and region
//! selection, filter building with the server's limits, and parsing of the
//! raw transaction bytes carried by every update.

pub mod connect;
pub mod feed;
pub mod filter;
pub mod parse;

pub use {
    connect::{Client, ConnectError, Connector},
    feed::{Feed, Region, RegionError},
    filter::{Filter, FilterError, Filters},
    triton_preconfs_proto as proto,
};
