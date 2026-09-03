//! Subscribes to one feed in one region and logs every event.
//!
//! ```text
//! preconfs-subscribe --endpoint https://preconfs.rpcpool.com --x-token $TOKEN \
//!     --region harmonic:ams --account TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA
//! ```

use {
    anyhow::Result,
    clap::Parser,
    solana_pubkey::Pubkey,
    solana_signature::Signature,
    tracing::{info, warn},
    triton_preconfs_client::{
        Connector, Event, Feed, Filter, Filters, Region, parse,
        proto::preconfs::{BamTransaction, ExecutionResult, HarmonicTransaction},
    },
};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "https://preconfs.rpcpool.com")]
    endpoint: String,
    #[arg(long, env = "PRECONFS_TOKEN")]
    x_token: Option<String>,
    /// host:port to connect to instead of resolving the endpoint (pins one
    /// point of presence behind the anycast address).
    #[arg(long)]
    dial: Option<String>,
    /// Feed and region, e.g. harmonic:ams or bam:fra.
    #[arg(long)]
    region: Region,
    /// Transactions referencing any of these accounts.
    #[arg(long = "account")]
    accounts: Vec<Pubkey>,
    /// Transactions referencing all of these accounts.
    #[arg(long = "require")]
    required: Vec<Pubkey>,
    #[arg(long = "signature")]
    signatures: Vec<Signature>,
    /// Harmonic only: success, execution_failure or fees_only.
    #[arg(long = "result")]
    results: Vec<ExecutionResult>,
    /// End the stream on the first disconnect instead of resubscribing.
    #[arg(long)]
    no_reconnect: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();
    let filter = Filter::new()
        .accounts(args.accounts)
        .require(args.required)
        .signatures(args.signatures)
        .execution_results(args.results);
    let filters = Filters::single(filter);

    let mut connector = Connector::new(&args.endpoint).x_token(args.x_token);
    if let Some(dial) = &args.dial {
        connector = connector.dial(dial);
    }
    if args.no_reconnect {
        connector = connector.no_reconnect();
    }
    let client = connector.connect().await?;
    let version = client.version().await?;
    info!(version = version.version, pop = version.region, "connected");

    match args.region.feed() {
        Feed::Harmonic => {
            let mut stream = client.subscribe_harmonic(args.region, filters).await?;
            while let Some(event) = stream.next().await {
                match event? {
                    Event::Transaction(matched) => {
                        log_harmonic(&matched.filters, &matched.transaction)
                    }
                    other => log_event(&other),
                }
            }
        }
        Feed::Bam => {
            let mut stream = client.subscribe_bam(args.region, filters).await?;
            while let Some(event) = stream.next().await {
                match event? {
                    Event::Transaction(matched) => log_bam(&matched.filters, &matched.transaction),
                    other => log_event(&other),
                }
            }
        }
    }
    info!("stream ended");
    Ok(())
}

fn log_harmonic(filters: &[String], txn: &HarmonicTransaction) {
    let signature = parse::parse_signature(&txn.transaction).ok();
    info!(
        slot = txn.slot,
        region = txn.region,
        seq = txn.seq,
        result = ?txn.result(),
        signature = ?signature,
        ?filters,
        "txn"
    );
}

fn log_bam(filters: &[String], txn: &BamTransaction) {
    let signature = parse::parse_signature(&txn.transaction).ok();
    info!(
        slot = txn.slot,
        node = txn.node,
        sequence = txn.sequence,
        revert_on_error = txn.is_revert_on_error,
        signature = ?signature,
        ?filters,
        "txn"
    );
}

fn log_event<T>(event: &Event<T>) {
    match event {
        Event::SlotStart { slot } => info!(slot, "slot start"),
        Event::SlotEnd { slot } => info!(slot, "slot end"),
        Event::Clip { transactions } => warn!(transactions, "clipped by coverage"),
        Event::Reconnected { attempts } => warn!(attempts, "reconnected, data in between is lost"),
        Event::Transaction(_) => {}
    }
}
