//! Subscribe filters, validated against the server's limits before the
//! request leaves the client, so a mistake fails here with a clear error
//! instead of an INVALID_ARGUMENT from the other side.

use {
    crate::feed::{Feed, Region},
    solana_pubkey::Pubkey,
    solana_signature::Signature,
    std::collections::HashMap,
    triton_preconfs_proto::preconfs::{ExecutionResult, SubscribeRequest, TransactionFilter},
};

/// Limits enforced by the server; a request over any of them is refused.
pub const MAX_FILTERS: usize = 64;
pub const MAX_ACCOUNTS_PER_LIST: usize = 10_000;
pub const MAX_SIGNATURES_PER_FILTER: usize = 1_000;
pub const MAX_FILTER_NAME_BYTES: usize = 64;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FilterError {
    #[error("at least one filter is required")]
    NoFilters,
    #[error("too many filters (max {MAX_FILTERS})")]
    TooManyFilters,
    #[error("filter name {0:?} is longer than {MAX_FILTER_NAME_BYTES} bytes")]
    NameTooLong(String),
    #[error("filter {0}: too many accounts (max {MAX_ACCOUNTS_PER_LIST})")]
    TooManyAccounts(String),
    #[error("filter {0}: too many signatures (max {MAX_SIGNATURES_PER_FILTER})")]
    TooManySignatures(String),
    #[error(
        "filter {0}: set account_include, account_required or signatures; full-feed subscriptions are refused"
    )]
    Empty(String),
    #[error("filter {0}: execution results are only available on the harmonic feed")]
    ExecutionResultsUnsupported(String),
}

/// One named filter. A transaction matches when it satisfies every set
/// condition: any of `account_include`, all of `account_required`, one of
/// `signatures`, one of `execution_results`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    pub account_include: Vec<Pubkey>,
    pub account_required: Vec<Pubkey>,
    pub signatures: Vec<Signature>,
    /// Harmonic only.
    pub execution_results: Vec<ExecutionResult>,
}

impl Filter {
    /// Transactions referencing any of these accounts.
    pub fn accounts(accounts: impl IntoIterator<Item = Pubkey>) -> Self {
        Self {
            account_include: accounts.into_iter().collect(),
            ..Self::default()
        }
    }

    pub fn require(mut self, accounts: impl IntoIterator<Item = Pubkey>) -> Self {
        self.account_required.extend(accounts);
        self
    }

    pub fn signatures(mut self, signatures: impl IntoIterator<Item = Signature>) -> Self {
        self.signatures.extend(signatures);
        self
    }

    pub fn execution_results(mut self, results: impl IntoIterator<Item = ExecutionResult>) -> Self {
        self.execution_results.extend(results);
        self
    }

    fn validate(&self, name: &str, feed: Feed) -> Result<(), FilterError> {
        if name.len() > MAX_FILTER_NAME_BYTES {
            return Err(FilterError::NameTooLong(name.to_string()));
        }
        if self.account_include.len() > MAX_ACCOUNTS_PER_LIST
            || self.account_required.len() > MAX_ACCOUNTS_PER_LIST
        {
            return Err(FilterError::TooManyAccounts(name.to_string()));
        }
        if self.signatures.len() > MAX_SIGNATURES_PER_FILTER {
            return Err(FilterError::TooManySignatures(name.to_string()));
        }
        if self.account_include.is_empty()
            && self.account_required.is_empty()
            && self.signatures.is_empty()
        {
            return Err(FilterError::Empty(name.to_string()));
        }
        if !self.execution_results.is_empty() && !feed.has_execution_results() {
            return Err(FilterError::ExecutionResultsUnsupported(name.to_string()));
        }
        Ok(())
    }

    fn into_proto(self) -> TransactionFilter {
        TransactionFilter {
            account_include: self.account_include.iter().map(Pubkey::to_string).collect(),
            account_required: self
                .account_required
                .iter()
                .map(Pubkey::to_string)
                .collect(),
            signature: self.signatures.iter().map(Signature::to_string).collect(),
            execution_results: self.execution_results.iter().map(|r| *r as i32).collect(),
        }
    }
}

/// Named filters for one stream; every matching update echoes the names.
#[derive(Debug, Clone, Default)]
pub struct Filters {
    filters: Vec<(String, Filter)>,
}

impl Filters {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, name: impl Into<String>, filter: Filter) -> Self {
        self.filters.push((name.into(), filter));
        self
    }

    /// A single filter named `default`.
    pub fn single(filter: Filter) -> Self {
        Self::new().with("default", filter)
    }

    /// Validates against the server's limits and builds the request.
    pub fn into_request(self, region: Region) -> Result<SubscribeRequest, FilterError> {
        if self.filters.is_empty() {
            return Err(FilterError::NoFilters);
        }
        if self.filters.len() > MAX_FILTERS {
            return Err(FilterError::TooManyFilters);
        }
        let feed = region.feed();
        let mut transactions = HashMap::with_capacity(self.filters.len());
        for (name, filter) in self.filters {
            filter.validate(&name, feed)?;
            transactions.insert(name, filter.into_proto());
        }
        Ok(SubscribeRequest {
            transactions,
            region: Some(region.into_proto()),
        })
    }
}

#[cfg(test)]
mod tests {
    use {super::*, triton_preconfs_proto::preconfs::HarmonicRegion};

    fn key(byte: u8) -> Pubkey {
        Pubkey::new_from_array([byte; 32])
    }

    #[test]
    fn builds_the_request_the_server_expects() {
        let region = Region::Harmonic(HarmonicRegion::Ams);
        let request = Filters::new()
            .with("mine", Filter::accounts([key(1)]).require([key(2)]))
            .with(
                "landed",
                Filter::accounts([key(3)]).execution_results([ExecutionResult::Success]),
            )
            .into_request(region)
            .unwrap();
        assert_eq!(request.transactions.len(), 2);
        let mine = &request.transactions["mine"];
        assert_eq!(mine.account_include, vec![key(1).to_string()]);
        assert_eq!(mine.account_required, vec![key(2).to_string()]);
        assert_eq!(request.transactions["landed"].execution_results, vec![0]);
        assert!(matches!(
            request.region,
            Some(triton_preconfs_proto::preconfs::subscribe_request::Region::HarmonicRegion(1))
        ));
    }

    #[test]
    fn limits_and_feed_rules_fail_before_the_round_trip() {
        let bam = Region::parse(Feed::Bam, "fra").unwrap();
        assert_eq!(
            Filters::new().into_request(bam).unwrap_err(),
            FilterError::NoFilters
        );
        assert_eq!(
            Filters::single(Filter::default())
                .into_request(bam)
                .unwrap_err(),
            FilterError::Empty("default".into())
        );
        assert_eq!(
            Filters::single(
                Filter::accounts([key(1)]).execution_results([ExecutionResult::Success])
            )
            .into_request(bam)
            .unwrap_err(),
            FilterError::ExecutionResultsUnsupported("default".into())
        );
        assert_eq!(
            Filters::new()
                .with("n".repeat(65), Filter::accounts([key(1)]))
                .into_request(bam)
                .unwrap_err(),
            FilterError::NameTooLong("n".repeat(65))
        );
        let mut many = Filters::new();
        for index in 0..65 {
            many = many.with(format!("f{index}"), Filter::accounts([key(1)]));
        }
        assert_eq!(
            many.into_request(bam).unwrap_err(),
            FilterError::TooManyFilters
        );
    }
}
