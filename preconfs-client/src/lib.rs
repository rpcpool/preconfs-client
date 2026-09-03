//! Client for the Triton Preconfs streams.
//!
//! Preconfs are preconfirmed Solana transactions: the Harmonic and BAM feeds
//! deliver them while the slot is still being built, before the transaction
//! lands on chain. This crate connects to a Triton Preconfs server, builds
//! validated filters and turns each stream into typed [`Event`]s, resubscribing
//! when a connection drops.
//!
//! ```no_run
//! use solana_pubkey::Pubkey;
//! use triton_preconfs_client::{Connector, Event, Feed, Filter, Filters, Region, parse};
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let client = Connector::new("https://preconfs.rpcpool.com")
//!     .x_token(Some("my-token"))
//!     .connect()
//!     .await?;
//!
//! let token_program: Pubkey = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".parse()?;
//! let region = Region::parse(Feed::Harmonic, "ams")?;
//! let filters = Filters::single(Filter::new().accounts([token_program]));
//!
//! let mut stream = client.subscribe_harmonic(region, filters).await?;
//! while let Some(event) = stream.next().await {
//!     match event? {
//!         Event::Transaction(matched) => {
//!             let signature = parse::parse_signature(&matched.transaction.transaction)?;
//!             println!("slot {} {signature}", matched.transaction.slot);
//!         }
//!         Event::SlotEnd { slot } => println!("slot {slot} complete"),
//!         Event::Reconnected { attempts } => println!("reconnected after {attempts} attempts"),
//!         _ => {}
//!     }
//! }
//! # Ok(()) }
//! ```
//!
//! # Streams
//!
//! A stream serves one feed in one region and yields [`Event`]s. On the
//! Harmonic feed every transaction sits between its slot's `SlotStart` and
//! `SlotEnd`; after `SlotEnd` the program holds everything its filters
//! matched for that slot. BAM has no framing, each transaction names its
//! slot. Pings are consumed by the stream. Coverage clipping is announced
//! with [`Event::Clip`], never silent.
//!
//! # Reconnect
//!
//! Points of presence restart on every deploy, so a long lived stream will
//! drop. By default the stream resubscribes with a backoff and yields
//! [`Event::Reconnected`] so the program knows it missed the data produced
//! in between (preconfs from the gap cannot be replayed). Errors that
//! retrying cannot fix end the stream: a bad token, a refused filter, a
//! region the server does not serve. Tune it with [`Connector::reconnect`]
//! or turn it off with [`Connector::no_reconnect`].
//!
//! # Errors
//!
//! One error type per step: [`ConnectError`] from connecting,
//! [`SubscribeError`] from opening a stream, [`StreamError`] from a stream
//! that ended. Wrap them with `anyhow` or `Box<dyn Error>` for one top
//! level type.

#![warn(missing_docs)]

pub mod connect;
pub mod error;
pub mod feed;
pub mod filter;
pub mod parse;
pub mod reconnect;
pub mod stream;

pub use {
    connect::{Client, Connector},
    error::{ConnectError, StreamError, SubscribeError},
    feed::{Feed, Region, RegionError},
    filter::{Filter, FilterError, Filters},
    reconnect::Reconnect,
    stream::{BamEvent, BamStream, Event, HarmonicEvent, HarmonicStream, Matched},
    triton_preconfs_proto as proto,
};
