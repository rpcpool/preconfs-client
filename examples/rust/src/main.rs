//! Subscribes to one feed in one region and logs every update.
//!
//!   preconfs-subscribe --endpoint https://preconfs.rpcpool.com --x-token $TOKEN \
//!       --feed harmonic --region ams --account TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA

use {
    anyhow::{Context, Result},
    clap::Parser,
    solana_pubkey::Pubkey,
    solana_signature::Signature,
    tracing::info,
    triton_preconfs_client::{
        Connector, Feed, Filter, Filters, Region, parse,
        proto::preconfs::{ExecutionResult, bam_update, harmonic_update},
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
    /// harmonic or bam.
    #[arg(long, default_value = "harmonic")]
    feed: String,
    /// Region of the feed, e.g. ams.
    #[arg(long)]
    region: String,
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
    results: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();
    let feed: Feed = args.feed.parse()?;
    let region = Region::parse(feed, &args.region)?;
    let results = args
        .results
        .iter()
        .map(|name| {
            ExecutionResult::from_str_name(&format!("EXECUTION_RESULT_{}", name.to_uppercase()))
                .with_context(|| format!("unknown execution result {name}"))
        })
        .collect::<Result<Vec<_>>>()?;
    let filter = Filter::accounts(args.accounts)
        .require(args.required)
        .signatures(args.signatures)
        .execution_results(results);
    let filters = Filters::single(filter);

    let mut connector = Connector::new(&args.endpoint);
    if let Some(token) = &args.x_token {
        connector = connector.x_token(token);
    }
    if let Some(dial) = &args.dial {
        connector = connector.dial(dial);
    }
    let client = connector.connect().await?;
    let version = client.version().await?;
    info!(version = version.version, pop = version.region, "connected");

    match feed {
        Feed::Harmonic => {
            let mut stream = client.subscribe_harmonic(region, filters).await?;
            while let Some(update) = stream.message().await? {
                match update.payload {
                    Some(harmonic_update::Payload::Transaction(txn)) => {
                        let signature = parse::parse_signature(&txn.transaction).ok();
                        info!(
                            slot = txn.slot,
                            region = txn.region,
                            seq = txn.seq,
                            result = txn.result,
                            signature = ?signature,
                            filters = ?update.filters,
                            "txn"
                        );
                    }
                    Some(harmonic_update::Payload::SlotStart(s)) => {
                        info!(slot = s.slot, "slot start")
                    }
                    Some(harmonic_update::Payload::SlotEnd(s)) => info!(slot = s.slot, "slot end"),
                    Some(harmonic_update::Payload::Ping(_)) => info!("ping"),
                    Some(harmonic_update::Payload::Clip(clip)) => {
                        info!(transactions = clip.transactions, "clipped by coverage");
                    }
                    None => {}
                }
            }
        }
        Feed::Bam => {
            let mut stream = client.subscribe_bam(region, filters).await?;
            while let Some(update) = stream.message().await? {
                match update.payload {
                    Some(bam_update::Payload::Transaction(txn)) => {
                        let signature = parse::parse_signature(&txn.transaction).ok();
                        info!(
                            slot = txn.slot,
                            node = txn.node,
                            sequence = txn.sequence,
                            revert_on_error = txn.is_revert_on_error,
                            signature = ?signature,
                            filters = ?update.filters,
                            "txn"
                        );
                    }
                    Some(bam_update::Payload::SlotStart(s)) => info!(slot = s.slot, "slot start"),
                    Some(bam_update::Payload::SlotEnd(s)) => info!(slot = s.slot, "slot end"),
                    Some(bam_update::Payload::Ping(_)) => info!("ping"),
                    Some(bam_update::Payload::Clip(clip)) => {
                        info!(transactions = clip.transactions, "clipped by coverage");
                    }
                    None => {}
                }
            }
        }
    }
    info!("stream ended");
    Ok(())
}
