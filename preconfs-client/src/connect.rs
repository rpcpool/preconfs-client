//! Connection to a Triton Preconfs server: TLS for https endpoints, the
//! `x-token` on every request, keepalive tuned for a stream that can be
//! quiet between leader windows, and an optional dial override to reach one
//! point of presence behind the anycast address.

use {
    crate::{
        feed::Region,
        filter::{FilterError, Filters},
    },
    std::time::Duration,
    tonic::{
        Request, Status, Streaming,
        codegen::InterceptedService,
        metadata::{Ascii, MetadataValue},
        transport::{Channel, ClientTlsConfig, Uri},
    },
    triton_preconfs_proto::preconfs::{
        BamUpdate, HarmonicUpdate, VersionRequest, VersionResponse, bam_client::BamClient,
        harmonic_client::HarmonicClient,
    },
};

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("endpoint is not a valid uri: {0}")]
    Uri(#[from] tonic::codegen::http::uri::InvalidUri),
    #[error("x-token is not valid ascii metadata")]
    Token,
    #[error("tls setup: {0}")]
    Tls(tonic::transport::Error),
    #[error("connect: {0}")]
    Transport(tonic::transport::Error),
    #[error(transparent)]
    Filter(#[from] FilterError),
    #[error(transparent)]
    Rpc(#[from] Status),
}

/// Adds the `x-token` to every request.
#[derive(Clone)]
pub struct TokenInterceptor {
    token: Option<MetadataValue<Ascii>>,
}

impl tonic::service::Interceptor for TokenInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        if let Some(token) = &self.token {
            request.metadata_mut().insert("x-token", token.clone());
        }
        Ok(request)
    }
}

/// How to reach the server. `preconfs.rpcpool.com` is anycast: the
/// connection lands on the closest point of presence; `dial` pins one by
/// opening the TCP connection to that address while TLS keeps the
/// endpoint's host name.
#[derive(Debug, Clone)]
pub struct Connector {
    endpoint: String,
    token: Option<String>,
    dial: Option<String>,
    connect_timeout: Duration,
}

impl Connector {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            token: None,
            dial: None,
            connect_timeout: Duration::from_secs(10),
        }
    }

    pub fn x_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    /// `host:port` to open the TCP connection to instead of resolving the
    /// endpoint host.
    pub fn dial(mut self, address: impl Into<String>) -> Self {
        self.dial = Some(address.into());
        self
    }

    pub const fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    pub async fn connect(self) -> Result<Client, ConnectError> {
        let uri: Uri = self.endpoint.parse()?;
        let token = self
            .token
            .as_deref()
            .map(|token| token.parse())
            .transpose()
            .map_err(|_| ConnectError::Token)?;
        let mut endpoint = Channel::builder(uri.clone())
            .connect_timeout(self.connect_timeout)
            .http2_keep_alive_interval(Duration::from_secs(30))
            .keep_alive_timeout(Duration::from_secs(10))
            .keep_alive_while_idle(true)
            // Slot bursts on long paths need more than the h2 default 65KB
            // stream window; let the transport size it from the BDP.
            .http2_adaptive_window(true);
        if uri.scheme_str() == Some("https") {
            endpoint = endpoint
                .tls_config(ClientTlsConfig::new().with_native_roots())
                .map_err(ConnectError::Tls)?;
        }
        let channel = match self.dial {
            None => endpoint.connect().await.map_err(ConnectError::Transport)?,
            Some(dial) => endpoint
                .connect_with_connector(tower::service_fn(move |_uri| {
                    let dial = dial.clone();
                    async move {
                        let stream = tokio::net::TcpStream::connect(dial).await?;
                        Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
                    }
                }))
                .await
                .map_err(ConnectError::Transport)?,
        };
        Ok(Client {
            channel,
            interceptor: TokenInterceptor { token },
        })
    }
}

pub type HarmonicStub = HarmonicClient<InterceptedService<Channel, TokenInterceptor>>;
pub type BamStub = BamClient<InterceptedService<Channel, TokenInterceptor>>;

/// A connected channel; streams and version calls are made from it. Cloning
/// shares the underlying HTTP/2 connection.
#[derive(Clone)]
pub struct Client {
    channel: Channel,
    interceptor: TokenInterceptor,
}

impl Client {
    pub fn harmonic(&self) -> HarmonicStub {
        HarmonicClient::with_interceptor(self.channel.clone(), self.interceptor.clone())
    }

    pub fn bam(&self) -> BamStub {
        BamClient::with_interceptor(self.channel.clone(), self.interceptor.clone())
    }

    /// Server version and the region of the point of presence answering.
    pub async fn version(&self) -> Result<VersionResponse, ConnectError> {
        Ok(self
            .harmonic()
            .get_version(VersionRequest {})
            .await?
            .into_inner())
    }

    /// One Harmonic stream: `region` must be a Harmonic region.
    pub async fn subscribe_harmonic(
        &self,
        region: Region,
        filters: Filters,
    ) -> Result<Streaming<HarmonicUpdate>, ConnectError> {
        let request = filters.into_request(region)?;
        Ok(self.harmonic().subscribe(request).await?.into_inner())
    }

    /// One BAM stream: `region` must be a BAM region.
    pub async fn subscribe_bam(
        &self,
        region: Region,
        filters: Filters,
    ) -> Result<Streaming<BamUpdate>, ConnectError> {
        let request = filters.into_request(region)?;
        Ok(self.bam().subscribe(request).await?.into_inner())
    }
}
