//! Error types, one per domain: building the connection, opening a stream,
//! and a stream failing. A program that wants one top level error wraps
//! them with `anyhow` or `Box<dyn Error>`.

use {
    crate::{feed::RegionError, filter::FilterError},
    tonic::Status,
};

/// Building the connection failed. From [`Connector::connect`](crate::Connector::connect)
/// and [`Connector::connect_lazy`](crate::Connector::connect_lazy).
#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    /// The endpoint is not a URI.
    #[error("endpoint is not a valid uri: {0}")]
    Uri(#[from] tonic::codegen::http::uri::InvalidUri),
    /// The x-token cannot be sent as gRPC metadata (non ascii).
    #[error("x-token is not valid ascii metadata")]
    Token,
    /// TLS could not be configured for the endpoint.
    #[error("tls setup: {0}")]
    Tls(#[source] tonic::transport::Error),
    /// The connection could not be established.
    #[error("connect: {0}")]
    Transport(#[source] tonic::transport::Error),
}

/// Opening a stream failed. From [`Client::subscribe_harmonic`](crate::Client::subscribe_harmonic)
/// and [`Client::subscribe_bam`](crate::Client::subscribe_bam).
#[derive(Debug, thiserror::Error)]
pub enum SubscribeError {
    /// The region does not belong to the feed being subscribed.
    #[error(transparent)]
    Region(#[from] RegionError),
    /// The filters break one of the server's limits.
    #[error(transparent)]
    Filter(#[from] FilterError),
    /// The server refused the subscribe: a bad token, a region it does not
    /// serve, a feed the token is not entitled to, or a limit on concurrent
    /// streams.
    #[error("subscribe refused: {0}")]
    Status(#[from] Status),
}

/// An open stream failed. Yielded once by the stream before it ends, or
/// swallowed when reconnect is on and retrying can fix it (see
/// [`Reconnect`](crate::Reconnect)).
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    /// The server ended the stream with a status; the code says why.
    #[error("stream ended: {0}")]
    Status(#[from] Status),
    /// The server closed the stream without a status.
    #[error("the server closed the stream")]
    Closed,
}
