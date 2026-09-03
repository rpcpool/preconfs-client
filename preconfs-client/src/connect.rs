//! Connecting to a Triton Preconfs server: TLS for https endpoints, the
//! `x-token` on every request, keepalive tuned for a stream that can be
//! quiet between leader windows, an optional dial override to reach one
//! point of presence behind the anycast address, and the reconnect policy
//! the streams follow.

use {
    crate::{
        error::{ConnectError, SubscribeError},
        feed::{Feed, Region, RegionError},
        filter::Filters,
        reconnect::Reconnect,
        stream::{BamStream, EventStream, FeedUpdate, HarmonicStream},
    },
    std::time::Duration,
    tonic::{
        Request, Status,
        codec::CompressionEncoding,
        codegen::InterceptedService,
        metadata::{Ascii, MetadataValue},
        transport::{Channel, ClientTlsConfig, Endpoint, Uri},
    },
    tonic_health::pb::{
        HealthCheckRequest, health_check_response::ServingStatus, health_client::HealthClient,
    },
    triton_preconfs_proto::preconfs::{
        BamUpdate, HarmonicUpdate, VersionRequest, VersionResponse, bam_client::BamClient,
        harmonic_client::HarmonicClient,
    },
};

/// Adds the `x-token` to every request.
#[derive(Debug, Clone)]
pub struct TokenInterceptor {
    token: Option<MetadataValue<Ascii>>,
}

impl tonic::service::Interceptor for TokenInterceptor {
    fn call(&mut self, mut request: Request<()>) -> std::result::Result<Request<()>, Status> {
        if let Some(token) = &self.token {
            request.metadata_mut().insert("x-token", token.clone());
        }
        Ok(request)
    }
}

/// How to reach the server and how its streams behave.
///
/// `preconfs.rpcpool.com` is anycast: the connection lands on the closest
/// point of presence. [`dial`](Self::dial) pins one by opening the TCP
/// connection to that address while TLS keeps the endpoint's host name.
///
/// Streams reconnect by default; see [`Reconnect`] and
/// [`no_reconnect`](Self::no_reconnect).
///
/// ```no_run
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// use triton_preconfs_client::Connector;
///
/// let client = Connector::new("https://preconfs.rpcpool.com")
///     .x_token(Some("my-token"))
///     .connect()
///     .await?;
/// # Ok(()) }
/// ```
#[derive(Debug, Clone)]
pub struct Connector {
    endpoint: String,
    token: Option<String>,
    dial: Option<String>,
    connect_timeout: Duration,
    tls: Option<ClientTlsConfig>,
    reconnect: Option<Reconnect>,
    compression: Option<CompressionEncoding>,
}

impl Connector {
    /// A connector for `endpoint`, such as `https://preconfs.rpcpool.com`.
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            token: None,
            dial: None,
            connect_timeout: Duration::from_secs(10),
            tls: None,
            reconnect: Some(Reconnect::default()),
            compression: None,
        }
    }

    /// The `x-token` sent with every request. `None` sends no token, which
    /// the server refuses unless it allows anonymous access.
    pub fn x_token(mut self, token: Option<impl Into<String>>) -> Self {
        self.token = token.map(Into::into);
        self
    }

    /// `host:port` to open the TCP connection to instead of resolving the
    /// endpoint host, to reach one point of presence behind the anycast
    /// address. TLS still verifies the endpoint's host name.
    pub fn dial(mut self, address: impl Into<String>) -> Self {
        self.dial = Some(address.into());
        self
    }

    /// Time allowed for the TCP and TLS handshake. Default 10s.
    pub const fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// TLS settings for an https endpoint. Default: the system's native
    /// root certificates.
    pub fn tls_config(mut self, tls: ClientTlsConfig) -> Self {
        self.tls = Some(tls);
        self
    }

    /// Retry schedule for resubscribing after a stream drops. Default
    /// [`Reconnect::default`].
    pub const fn reconnect(mut self, reconnect: Reconnect) -> Self {
        self.reconnect = Some(reconnect);
        self
    }

    /// Streams end with the error that dropped them instead of
    /// resubscribing.
    pub const fn no_reconnect(mut self) -> Self {
        self.reconnect = None;
        self
    }

    /// Ask the server to compress the streams. Needs the matching crate
    /// feature (`gzip` or `zstd`); without it the encoding is refused at
    /// runtime by tonic.
    pub const fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
        self.compression = Some(encoding);
        self
    }

    fn endpoint(&self) -> Result<Endpoint, ConnectError> {
        let uri: Uri = self.endpoint.parse()?;
        let mut endpoint = Channel::builder(uri.clone())
            .connect_timeout(self.connect_timeout)
            .http2_keep_alive_interval(Duration::from_secs(30))
            .keep_alive_timeout(Duration::from_secs(10))
            .keep_alive_while_idle(true)
            // Slot bursts on long paths need more than the h2 default 65KB
            // stream window; let the transport size it from the BDP.
            .http2_adaptive_window(true);
        if uri.scheme_str() == Some("https") {
            let tls = self
                .tls
                .clone()
                .unwrap_or_else(|| ClientTlsConfig::new().with_native_roots());
            endpoint = endpoint.tls_config(tls).map_err(ConnectError::Tls)?;
        }
        Ok(endpoint)
    }

    fn client(self, channel: Channel) -> Result<Client, ConnectError> {
        let token = self
            .token
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(|_| ConnectError::Token)?;
        Ok(Client {
            channel,
            interceptor: TokenInterceptor { token },
            reconnect: self.reconnect,
            compression: self.compression,
        })
    }

    fn dialer(
        dial: String,
    ) -> impl tower::Service<
        Uri,
        Response = hyper_util::rt::TokioIo<tokio::net::TcpStream>,
        Error = std::io::Error,
        Future = impl Send,
    > + Send
    + 'static {
        tower::service_fn(move |_uri| {
            let dial = dial.clone();
            async move {
                let stream = tokio::net::TcpStream::connect(dial).await?;
                Ok(hyper_util::rt::TokioIo::new(stream))
            }
        })
    }

    /// Connects and returns the client. Fails if the server cannot be
    /// reached within the connect timeout.
    pub async fn connect(self) -> Result<Client, ConnectError> {
        let endpoint = self.endpoint()?;
        let channel = match self.dial.clone() {
            None => endpoint.connect().await,
            Some(dial) => endpoint.connect_with_connector(Self::dialer(dial)).await,
        }
        .map_err(ConnectError::Transport)?;
        self.client(channel)
    }

    /// Returns the client without connecting; the connection is made by the
    /// first request.
    pub fn connect_lazy(self) -> Result<Client, ConnectError> {
        let endpoint = self.endpoint()?;
        let channel = match self.dial.clone() {
            None => endpoint.connect_lazy(),
            Some(dial) => endpoint.connect_with_connector_lazy(Self::dialer(dial)),
        };
        self.client(channel)
    }
}

/// The generated Harmonic client over the connection, for calls the typed
/// API does not cover.
pub type HarmonicStub = HarmonicClient<InterceptedService<Channel, TokenInterceptor>>;
/// The generated BAM client over the connection.
pub type BamStub = BamClient<InterceptedService<Channel, TokenInterceptor>>;

/// A connection to the server. Cloning shares the underlying HTTP/2
/// connection; streams and calls are made from it.
#[derive(Debug, Clone)]
pub struct Client {
    channel: Channel,
    interceptor: TokenInterceptor,
    reconnect: Option<Reconnect>,
    compression: Option<CompressionEncoding>,
}

impl Client {
    /// The generated Harmonic client with the token attached.
    pub fn harmonic(&self) -> HarmonicStub {
        let client =
            HarmonicClient::with_interceptor(self.channel.clone(), self.interceptor.clone());
        match self.compression {
            Some(encoding) => client.accept_compressed(encoding),
            None => client,
        }
    }

    /// The generated BAM client with the token attached.
    pub fn bam(&self) -> BamStub {
        let client = BamClient::with_interceptor(self.channel.clone(), self.interceptor.clone());
        match self.compression {
            Some(encoding) => client.accept_compressed(encoding),
            None => client,
        }
    }

    /// Server version and the region of the point of presence answering.
    pub async fn version(&self) -> Result<VersionResponse, Status> {
        Ok(self
            .harmonic()
            .get_version(VersionRequest {})
            .await?
            .into_inner())
    }

    /// The server's overall health, from the standard gRPC health service.
    pub async fn health(&self) -> Result<ServingStatus, Status> {
        let response = HealthClient::new(self.channel.clone())
            .check(HealthCheckRequest {
                service: String::new(),
            })
            .await?
            .into_inner();
        Ok(ServingStatus::try_from(response.status).unwrap_or(ServingStatus::Unknown))
    }

    /// Subscribes to the Harmonic feed in `region` with `filters`. The
    /// filters are validated here; the region must be a Harmonic region.
    pub async fn subscribe_harmonic(
        &self,
        region: Region,
        filters: Filters,
    ) -> Result<HarmonicStream, SubscribeError> {
        self.subscribe::<HarmonicUpdate>(Feed::Harmonic, region, filters)
            .await
    }

    /// Subscribes to the BAM feed in `region` with `filters`. The filters
    /// are validated here; the region must be a BAM region.
    pub async fn subscribe_bam(
        &self,
        region: Region,
        filters: Filters,
    ) -> Result<BamStream, SubscribeError> {
        self.subscribe::<BamUpdate>(Feed::Bam, region, filters)
            .await
    }

    /// The first subscribe is not retried: its error is the caller's to
    /// see. Reconnects happen inside the stream.
    async fn subscribe<U: FeedUpdate>(
        &self,
        feed: Feed,
        region: Region,
        filters: Filters,
    ) -> Result<EventStream<U>, SubscribeError> {
        if region.feed() != feed {
            return Err(RegionError::WrongFeed {
                expected: feed,
                region,
            }
            .into());
        }
        let request = filters.into_request(region)?;
        let stream = U::subscribe(self.clone(), request.clone()).await?;
        Ok(EventStream::new(
            self.clone(),
            request,
            self.reconnect.clone(),
            stream,
        ))
    }
}
