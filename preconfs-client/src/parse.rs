//! Transaction-byte parsing: the first signature and the static account keys
//! the filters and the monitors need, from the raw bytes of a legacy, v0 or
//! v1 (SIMD-0385) transaction. The bytes must be one whole transaction, as
//! on the wire.

use {
    solana_pubkey::Pubkey, solana_signature::Signature,
    solana_transaction::versioned::VersionedTransaction, thiserror::Error,
};

/// Bytes that are not a transaction.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ParseError {
    /// Not a transaction: truncated, trailing bytes or an unknown version.
    /// Keys read from the wrong offsets would match nothing or the wrong
    /// filters, so this is an error, not a guess.
    #[error("malformed transaction: {0}")]
    Malformed(#[source] wincode::ReadError),
    /// A transaction with zero signatures.
    #[error("transaction has no signatures")]
    NoSignature,
}

/// Decodes raw transaction bytes.
pub fn parse(data: &[u8]) -> Result<VersionedTransaction, ParseError> {
    wincode::deserialize_exact(data).map_err(ParseError::Malformed)
}

fn first_signature(transaction: &VersionedTransaction) -> Result<Signature, ParseError> {
    transaction
        .signatures
        .first()
        .copied()
        .ok_or(ParseError::NoSignature)
}

/// Extracts the first signature from raw transaction bytes.
pub fn parse_signature(data: &[u8]) -> Result<Signature, ParseError> {
    first_signature(&parse(data)?)
}

/// Extracts the first signature and the static account keys from raw
/// transaction bytes. Addresses a v0 transaction loads through lookup tables
/// are not included.
pub fn parse_static_parts(data: &[u8]) -> Result<(Signature, Vec<Pubkey>), ParseError> {
    let transaction = parse(data)?;
    Ok((
        first_signature(&transaction)?,
        transaction.message.static_account_keys().to_vec(),
    ))
}
