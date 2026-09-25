//! Subscribes to one feed in one region and logs every event.
//!
//! ```text
//! preconfs-subscribe --endpoint https://preconfs.rpcpool.com --x-token $TOKEN \
//!     --region harmonic:ams --account TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA
//!
//! # Meteora DBC create_config from any creator, by its Anchor discriminator
//! preconfs-subscribe --x-token $TOKEN --region bam:fra \
//!     --instruction dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN:0:c9cff3724b6f2fbd
//! ```

use {
    anyhow::Result,
    clap::Parser,
    solana_pubkey::Pubkey,
    solana_signature::Signature,
    tracing::{info, warn},
    triton_preconfs_client::{
        Connector, Event, Feed, Filter, Filters, InstructionFilter, Region, parse,
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
    /// Drops transactions referencing any of these accounts.
    #[arg(long = "exclude")]
    excluded: Vec<Pubkey>,
    /// Transactions signed by any of these accounts.
    #[arg(long = "signer")]
    signers: Vec<Pubkey>,
    /// Drops transactions signed by any of these accounts.
    #[arg(long = "exclude-signer")]
    excluded_signers: Vec<Pubkey>,
    /// A top-level instruction invoking PROGRAM, optionally with the hex
    /// bytes HEX at OFFSET of its data: PROGRAM or PROGRAM:OFFSET:HEX.
    /// Repeat for any of several.
    #[arg(long = "instruction", value_parser = parse_instruction)]
    instructions: Vec<InstructionFilter>,
    /// Exact data length for every --instruction.
    #[arg(long, requires = "instructions")]
    data_size: Option<u32>,
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
    let instructions = args
        .instructions
        .into_iter()
        .map(|instruction| match args.data_size {
            Some(size) => instruction.data_size(size),
            None => instruction,
        });
    let filter = Filter::new()
        .accounts(args.accounts)
        .require(args.required)
        .exclude_accounts(args.excluded)
        .signers(args.signers)
        .exclude_signers(args.excluded_signers)
        .instructions(instructions)
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
        // Transactions are logged by the caller; new event kinds are ignored.
        _ => {}
    }
}

/// PROGRAM or PROGRAM:OFFSET:HEX.
fn parse_instruction(spec: &str) -> Result<InstructionFilter, String> {
    let mut parts = spec.split(':');
    let program: Pubkey = parts
        .next()
        .unwrap_or_default()
        .parse()
        .map_err(|_| format!("{spec}: bad program id"))?;
    let filter = InstructionFilter::new(program);
    match (parts.next(), parts.next(), parts.next()) {
        (None, _, _) => Ok(filter),
        (Some(offset), Some(hex), None) => {
            let offset = offset.parse().map_err(|_| format!("{spec}: bad offset"))?;
            Ok(filter.memcmp(offset, decode_hex(hex).ok_or(format!("{spec}: bad hex"))?))
        }
        _ => Err(format!("{spec}: expected PROGRAM or PROGRAM:OFFSET:HEX")),
    }
}

/// Pairs of hex digits, read as bytes so a multibyte character is an error
/// rather than a split in the middle of it.
fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    hex.as_bytes()
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DBC: &str = "dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN";

    #[test]
    fn instruction_specs() {
        let program: Pubkey = DBC.parse().unwrap();
        assert_eq!(
            parse_instruction(DBC).unwrap(),
            InstructionFilter::new(program)
        );
        assert_eq!(
            parse_instruction(&format!("{DBC}:0:c9cff3724b6f2fbd")).unwrap(),
            InstructionFilter::new(program).memcmp(0, [201, 207, 243, 114, 75, 111, 47, 189])
        );
        assert_eq!(
            parse_instruction(&format!("{DBC}:8:0A")).unwrap(),
            InstructionFilter::new(program).memcmp(8, [10])
        );
        for bad in [
            String::new(),
            "not-a-key".to_string(),
            format!("{DBC}:"),
            format!("{DBC}:0"),
            format!("{DBC}:x:00"),
            format!("{DBC}:-1:00"),
            format!("{DBC}:0:0"),
            format!("{DBC}:0:zz"),
            format!("{DBC}:0:\u{e9}\u{e9}"),
            format!("{DBC}:0:00:00"),
        ] {
            assert!(parse_instruction(&bad).is_err(), "{bad:?}");
        }
    }
}
