//! Subscribe filters, validated against the server's limits before the
//! request leaves the client, so a mistake fails here with a clear error
//! instead of an INVALID_ARGUMENT from the other side.

use {
    crate::feed::{Feed, Region},
    solana_pubkey::Pubkey,
    solana_signature::Signature,
    std::collections::HashMap,
    triton_preconfs_proto::preconfs::{
        self as proto, ExecutionResult, SubscribeRequest, TransactionFilter,
    },
};

/// Filters per stream; a request over it is refused.
pub const MAX_FILTERS: usize = 64;
/// Accounts per `account_include`, `account_exclude`, `account_required`,
/// `signer_include` or `signer_exclude` list.
pub const MAX_ACCOUNTS_PER_LIST: usize = 10_000;
/// Signatures per filter.
pub const MAX_SIGNATURES_PER_FILTER: usize = 1_000;
/// Bytes in a filter name; names are echoed on every matching update.
pub const MAX_FILTER_NAME_BYTES: usize = 64;
/// Instruction filters per stream, summed over all its filters: the limit is
/// on the stream, not on one filter.
pub const MAX_INSTRUCTION_FILTERS: usize = 16;
/// Memcmps per instruction filter, as getProgramAccounts allows four filters.
pub const MAX_MEMCMPS_PER_INSTRUCTION: usize = 4;
/// Bytes per memcmp, the getProgramAccounts limit.
pub const MAX_MEMCMP_BYTES: usize = 128;
/// Largest instruction data a transaction can carry: a v1 transaction is at
/// most 4096 bytes. A memcmp or data size past it can never match.
pub const MAX_INSTRUCTION_DATA_BYTES: usize = 4096;

/// A filter set the server would refuse.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum FilterError {
    /// The set is empty.
    #[error("at least one filter is required")]
    NoFilters,
    /// More than [`MAX_FILTERS`].
    #[error("too many filters (max {MAX_FILTERS})")]
    TooManyFilters,
    /// A name longer than [`MAX_FILTER_NAME_BYTES`].
    #[error("filter name {0:?} is longer than {MAX_FILTER_NAME_BYTES} bytes")]
    NameTooLong(String),
    /// An account list longer than [`MAX_ACCOUNTS_PER_LIST`].
    #[error("filter {0}: too many accounts (max {MAX_ACCOUNTS_PER_LIST})")]
    TooManyAccounts(String),
    /// More signatures than [`MAX_SIGNATURES_PER_FILTER`].
    #[error("filter {0}: too many signatures (max {MAX_SIGNATURES_PER_FILTER})")]
    TooManySignatures(String),
    /// More instruction filters on the stream than [`MAX_INSTRUCTION_FILTERS`].
    #[error("too many instruction filters (max {MAX_INSTRUCTION_FILTERS} per stream)")]
    TooManyInstructionFilters,
    /// An instruction filter with more than [`MAX_MEMCMPS_PER_INSTRUCTION`]
    /// memcmps.
    #[error("filter {0}: too many memcmps on an instruction (max {MAX_MEMCMPS_PER_INSTRUCTION})")]
    TooManyMemcmps(String),
    /// A memcmp with no bytes or more than [`MAX_MEMCMP_BYTES`].
    #[error("filter {0}: memcmp bytes must be 1 to {MAX_MEMCMP_BYTES} long")]
    MemcmpLength(String),
    /// A memcmp that ends past the instruction's `data_size` or past
    /// [`MAX_INSTRUCTION_DATA_BYTES`]; it could never match.
    #[error("filter {0}: memcmp reaches past the instruction data")]
    MemcmpOutOfRange(String),
    /// A `data_size` over [`MAX_INSTRUCTION_DATA_BYTES`].
    #[error("filter {0}: data_size over {MAX_INSTRUCTION_DATA_BYTES}")]
    DataSizeTooLarge(String),
    /// A filter that selects nothing; the full feed cannot be subscribed.
    #[error(
        "filter {0}: set account_include, account_required, signer_include, instructions or signatures (the exclude lists only narrow them); full-feed subscriptions are refused"
    )]
    Empty(String),
    /// `execution_results` on a feed that does not report them.
    #[error("filter {0}: execution results are only available on the harmonic feed")]
    ExecutionResultsUnsupported(String),
}

/// One named filter. A transaction matches when it satisfies every set
/// condition: any of `account_include`, all of `account_required`, none of
/// `account_exclude`, signed by any of `signer_include` and none of
/// `signer_exclude`, any of `instructions`, one of `signatures`, one of
/// `execution_results`.
///
/// ```
/// use triton_preconfs_client::{Filter, InstructionFilter};
/// use solana_pubkey::Pubkey;
///
/// let token_program: Pubkey = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".parse().unwrap();
/// let mine = Pubkey::new_unique();
/// let filter = Filter::new().accounts([token_program]).require([mine]);
/// // SPL token transfers (instruction 3) signed by `mine`.
/// let transfers = Filter::new()
///     .signers([mine])
///     .instructions([InstructionFilter::new(token_program).memcmp(0, [3]).data_size(9)]);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Filter {
    /// Matches transactions referencing any of these accounts.
    pub account_include: Vec<Pubkey>,
    /// Drops transactions referencing any of these accounts.
    pub account_exclude: Vec<Pubkey>,
    /// Matches transactions referencing all of these accounts.
    pub account_required: Vec<Pubkey>,
    /// Matches transactions signed by any of these accounts.
    pub signer_include: Vec<Pubkey>,
    /// Drops transactions signed by any of these accounts.
    pub signer_exclude: Vec<Pubkey>,
    /// Matches transactions with a top-level instruction passing any of these.
    pub instructions: Vec<InstructionFilter>,
    /// Matches these transactions by first signature.
    pub signatures: Vec<Signature>,
    /// Matches transactions with one of these outcomes. Harmonic only.
    pub execution_results: Vec<ExecutionResult>,
}

impl Filter {
    /// An empty filter; add at least one condition before subscribing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds accounts any of which a transaction must reference.
    pub fn accounts(mut self, accounts: impl IntoIterator<Item = Pubkey>) -> Self {
        self.account_include.extend(accounts);
        self
    }

    /// Adds accounts all of which a transaction must reference.
    pub fn require(mut self, accounts: impl IntoIterator<Item = Pubkey>) -> Self {
        self.account_required.extend(accounts);
        self
    }

    /// Adds accounts none of which a transaction may reference. Narrows a
    /// selection; a filter with only exclusions is refused by
    /// [`Filters::into_request`].
    pub fn exclude_accounts(mut self, accounts: impl IntoIterator<Item = Pubkey>) -> Self {
        self.account_exclude.extend(accounts);
        self
    }

    /// Adds accounts any of which must have signed a transaction. The
    /// signers are the first `num_required_signatures` static account keys,
    /// the fee payer among them.
    pub fn signers(mut self, signers: impl IntoIterator<Item = Pubkey>) -> Self {
        self.signer_include.extend(signers);
        self
    }

    /// Adds accounts none of which may have signed a transaction. Narrows a
    /// selection like [`exclude_accounts`](Self::exclude_accounts).
    pub fn exclude_signers(mut self, signers: impl IntoIterator<Item = Pubkey>) -> Self {
        self.signer_exclude.extend(signers);
        self
    }

    /// Adds instruction filters any of which a top-level instruction must
    /// pass.
    pub fn instructions(
        mut self,
        instructions: impl IntoIterator<Item = InstructionFilter>,
    ) -> Self {
        self.instructions.extend(instructions);
        self
    }

    /// Adds signatures to match on.
    pub fn signatures(mut self, signatures: impl IntoIterator<Item = Signature>) -> Self {
        self.signatures.extend(signatures);
        self
    }

    /// Adds execution outcomes to match on. Harmonic only.
    pub fn execution_results(mut self, results: impl IntoIterator<Item = ExecutionResult>) -> Self {
        self.execution_results.extend(results);
        self
    }

    fn validate(&self, name: &str, feed: Feed) -> Result<(), FilterError> {
        if name.len() > MAX_FILTER_NAME_BYTES {
            return Err(FilterError::NameTooLong(name.to_string()));
        }
        if [
            &self.account_include,
            &self.account_exclude,
            &self.account_required,
            &self.signer_include,
            &self.signer_exclude,
        ]
        .iter()
        .any(|list| list.len() > MAX_ACCOUNTS_PER_LIST)
        {
            return Err(FilterError::TooManyAccounts(name.to_string()));
        }
        if self.signatures.len() > MAX_SIGNATURES_PER_FILTER {
            return Err(FilterError::TooManySignatures(name.to_string()));
        }
        for instruction in &self.instructions {
            instruction.validate(name)?;
        }
        if self.account_include.is_empty()
            && self.account_required.is_empty()
            && self.signer_include.is_empty()
            && self.instructions.is_empty()
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
            account_exclude: self.account_exclude.iter().map(Pubkey::to_string).collect(),
            account_required: self
                .account_required
                .iter()
                .map(Pubkey::to_string)
                .collect(),
            signer_include: self.signer_include.iter().map(Pubkey::to_string).collect(),
            signer_exclude: self.signer_exclude.iter().map(Pubkey::to_string).collect(),
            instructions: self
                .instructions
                .into_iter()
                .map(InstructionFilter::into_proto)
                .collect(),
            signature: self.signatures.iter().map(Signature::to_string).collect(),
            execution_results: self
                .execution_results
                .iter()
                .copied()
                .map(i32::from)
                .collect(),
        }
    }
}

/// A condition on one top-level instruction, modelled on the
/// getProgramAccounts filters: the instruction invokes `program_id` and its
/// data passes every memcmp and the data size, when set. Instructions a
/// program invokes through CPI are not part of the transaction and are not
/// seen.
///
/// ```
/// use triton_preconfs_client::InstructionFilter;
/// use solana_pubkey::Pubkey;
///
/// let program = Pubkey::new_unique();
/// // An Anchor instruction by its 8 byte discriminator.
/// let swap = InstructionFilter::new(program).memcmp(0, [248, 198, 158, 145, 225, 117, 135, 200]);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct InstructionFilter {
    /// Program the instruction invokes.
    pub program_id: Pubkey,
    /// Bytes the instruction data must hold at their offsets.
    pub memcmp: Vec<Memcmp>,
    /// Exact length of the instruction data.
    pub data_size: Option<u32>,
}

/// Bytes at an offset of the instruction data. Data shorter than the offset
/// plus the bytes does not match.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Memcmp {
    /// Offset into the instruction data.
    pub offset: u32,
    /// Bytes expected at `offset`.
    pub bytes: Vec<u8>,
}

impl InstructionFilter {
    /// Instructions invoking `program_id`, any data.
    pub const fn new(program_id: Pubkey) -> Self {
        Self {
            program_id,
            memcmp: Vec::new(),
            data_size: None,
        }
    }

    /// Requires `bytes` at `offset` of the instruction data.
    pub fn memcmp(mut self, offset: u32, bytes: impl Into<Vec<u8>>) -> Self {
        self.memcmp.push(Memcmp {
            offset,
            bytes: bytes.into(),
        });
        self
    }

    /// Requires the instruction data to be exactly `size` bytes.
    pub const fn data_size(mut self, size: u32) -> Self {
        self.data_size = Some(size);
        self
    }

    fn validate(&self, name: &str) -> Result<(), FilterError> {
        if self.memcmp.len() > MAX_MEMCMPS_PER_INSTRUCTION {
            return Err(FilterError::TooManyMemcmps(name.to_string()));
        }
        let data_bytes = match self.data_size {
            Some(size) if size as usize > MAX_INSTRUCTION_DATA_BYTES => {
                return Err(FilterError::DataSizeTooLarge(name.to_string()));
            }
            Some(size) => size as usize,
            None => MAX_INSTRUCTION_DATA_BYTES,
        };
        for memcmp in &self.memcmp {
            if memcmp.bytes.is_empty() || memcmp.bytes.len() > MAX_MEMCMP_BYTES {
                return Err(FilterError::MemcmpLength(name.to_string()));
            }
            // Checked: on a 32 bit target an offset near u32::MAX plus the
            // length would wrap and pass.
            let end = usize::try_from(memcmp.offset)
                .ok()
                .and_then(|offset| offset.checked_add(memcmp.bytes.len()));
            if end.is_none_or(|end| end > data_bytes) {
                return Err(FilterError::MemcmpOutOfRange(name.to_string()));
            }
        }
        Ok(())
    }

    fn into_proto(self) -> proto::InstructionFilter {
        proto::InstructionFilter {
            program_id: self.program_id.to_string(),
            memcmp: self
                .memcmp
                .into_iter()
                .map(|memcmp| proto::Memcmp {
                    offset: memcmp.offset,
                    bytes: memcmp.bytes.into(),
                })
                .collect(),
            data_size: self.data_size,
        }
    }
}

/// Named filters for one stream; every matching update echoes the names
/// that matched.
#[derive(Debug, Clone, Default)]
pub struct Filters {
    filters: Vec<(String, Filter)>,
}

impl Filters {
    /// An empty set; add filters with [`with`](Self::with).
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `filter` under `name`. Names are echoed on matching updates.
    pub fn with(mut self, name: impl Into<String>, filter: Filter) -> Self {
        self.filters.push((name.into(), filter));
        self
    }

    /// A set of one filter named `default`.
    pub fn single(filter: Filter) -> Self {
        Self::new().with("default", filter)
    }

    /// Number of filters in the set.
    pub const fn len(&self) -> usize {
        self.filters.len()
    }

    /// Whether the set has no filters.
    pub const fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }

    /// Validates against the server's limits and builds the request.
    pub fn into_request(self, region: Region) -> Result<SubscribeRequest, FilterError> {
        if self.filters.is_empty() {
            return Err(FilterError::NoFilters);
        }
        if self.filters.len() > MAX_FILTERS {
            return Err(FilterError::TooManyFilters);
        }
        let instructions: usize = self
            .filters
            .iter()
            .map(|(_, filter)| filter.instructions.len())
            .sum();
        if instructions > MAX_INSTRUCTION_FILTERS {
            return Err(FilterError::TooManyInstructionFilters);
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
            .with("mine", Filter::new().accounts([key(1)]).require([key(2)]))
            .with(
                "landed",
                Filter::new()
                    .accounts([key(3)])
                    .execution_results([ExecutionResult::Success]),
            )
            .into_request(region)
            .unwrap();
        assert_eq!(request.transactions.len(), 2);
        let mine = &request.transactions["mine"];
        assert_eq!(mine.account_include, vec![key(1).to_string()]);
        assert_eq!(mine.account_required, vec![key(2).to_string()]);
        let narrowed = Filters::single(Filter::new().accounts([key(1)]).exclude_accounts([key(9)]))
            .into_request(region)
            .unwrap();
        assert_eq!(
            narrowed.transactions["default"].account_exclude,
            vec![key(9).to_string()]
        );
        assert_eq!(
            Filters::single(Filter::new().exclude_accounts([key(9)]))
                .into_request(region)
                .unwrap_err(),
            FilterError::Empty("default".into()),
            "exclusions alone are not a selection"
        );
        let signed = Filters::single(
            Filter::new()
                .signers([key(4)])
                .exclude_signers([key(5)])
                .instructions([InstructionFilter::new(key(6))
                    .memcmp(1, [2, 3])
                    .data_size(9)]),
        )
        .into_request(region)
        .unwrap();
        let signed = &signed.transactions["default"];
        assert_eq!(signed.signer_include, vec![key(4).to_string()]);
        assert_eq!(signed.signer_exclude, vec![key(5).to_string()]);
        assert_eq!(
            signed.instructions,
            vec![proto::InstructionFilter {
                program_id: key(6).to_string(),
                memcmp: vec![proto::Memcmp {
                    offset: 1,
                    bytes: vec![2, 3].into(),
                }],
                data_size: Some(9),
            }]
        );
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
            Filters::single(Filter::new())
                .into_request(bam)
                .unwrap_err(),
            FilterError::Empty("default".into())
        );
        assert_eq!(
            Filters::single(
                Filter::new()
                    .accounts([key(1)])
                    .execution_results([ExecutionResult::Success])
            )
            .into_request(bam)
            .unwrap_err(),
            FilterError::ExecutionResultsUnsupported("default".into())
        );
        assert_eq!(
            Filters::new()
                .with("n".repeat(65), Filter::new().accounts([key(1)]))
                .into_request(bam)
                .unwrap_err(),
            FilterError::NameTooLong("n".repeat(65))
        );
        assert_eq!(
            Filters::single(
                Filter::new()
                    .accounts([key(1)])
                    .exclude_accounts((0..=MAX_ACCOUNTS_PER_LIST).map(|_| key(2)))
            )
            .into_request(bam)
            .unwrap_err(),
            FilterError::TooManyAccounts("default".into())
        );
        let program = key(7);
        for (instruction, error) in [
            (
                InstructionFilter::new(program)
                    .memcmp(0, [1])
                    .memcmp(1, [1])
                    .memcmp(2, [1])
                    .memcmp(3, [1])
                    .memcmp(4, [1]),
                FilterError::TooManyMemcmps("default".into()),
            ),
            (
                InstructionFilter::new(program).memcmp(0, []),
                FilterError::MemcmpLength("default".into()),
            ),
            (
                InstructionFilter::new(program).memcmp(0, vec![1; MAX_MEMCMP_BYTES + 1]),
                FilterError::MemcmpLength("default".into()),
            ),
            (
                InstructionFilter::new(program).memcmp(u32::MAX, [1]),
                FilterError::MemcmpOutOfRange("default".into()),
            ),
            (
                InstructionFilter::new(program).memcmp(8, [1]).data_size(8),
                FilterError::MemcmpOutOfRange("default".into()),
            ),
            (
                InstructionFilter::new(program).data_size(4097),
                FilterError::DataSizeTooLarge("default".into()),
            ),
        ] {
            assert_eq!(
                Filters::single(Filter::new().instructions([instruction]))
                    .into_request(bam)
                    .unwrap_err(),
                error
            );
        }
        // The last byte of the largest instruction data is still reachable.
        assert!(
            Filters::single(
                Filter::new().instructions([InstructionFilter::new(program).memcmp(4095, [1])])
            )
            .into_request(bam)
            .is_ok()
        );
        let mut spread = Filters::new();
        for index in 0..=MAX_INSTRUCTION_FILTERS / 4 {
            spread = spread.with(
                format!("f{index}"),
                Filter::new().instructions((0..4).map(|_| InstructionFilter::new(program))),
            );
        }
        assert_eq!(
            spread.into_request(bam).unwrap_err(),
            FilterError::TooManyInstructionFilters,
            "the instruction limit is per stream, not per filter"
        );
        assert_eq!(
            Filters::single(Filter::new().exclude_signers([key(1)]))
                .into_request(bam)
                .unwrap_err(),
            FilterError::Empty("default".into()),
            "signer exclusions alone are not a selection"
        );
        assert!(Filters::new().is_empty());
        assert!(!Filters::single(Filter::new()).is_empty());
        let mut many = Filters::new();
        for index in 0..65 {
            many = many.with(format!("f{index}"), Filter::new().accounts([key(1)]));
        }
        assert_eq!(many.len(), 65);
        assert_eq!(
            many.into_request(bam).unwrap_err(),
            FilterError::TooManyFilters
        );
    }

    fn one(filter: Filter) -> Result<SubscribeRequest, FilterError> {
        Filters::single(filter).into_request(Region::Harmonic(HarmonicRegion::Ams))
    }

    fn keys(count: usize) -> Vec<Pubkey> {
        (0..count)
            .map(|index| {
                let mut key = [0u8; 32];
                key[..8].copy_from_slice(&index.to_le_bytes());
                Pubkey::new_from_array(key)
            })
            .collect()
    }

    /// Every limit on both sides: the value at the limit is accepted, one
    /// past it is refused with the error that names it.
    #[test]
    fn every_limit_accepts_its_bound_and_refuses_one_past() {
        let base = || Filter::new().accounts([key(1)]);
        let at = MAX_ACCOUNTS_PER_LIST;
        type Add = fn(Filter, Vec<Pubkey>) -> Filter;
        let lists: [(&str, Add); 5] = [
            ("account_include", |filter, keys| filter.accounts(keys)),
            ("account_exclude", |filter, keys| {
                filter.exclude_accounts(keys)
            }),
            ("account_required", |filter, keys| filter.require(keys)),
            ("signer_include", |filter, keys| filter.signers(keys)),
            ("signer_exclude", |filter, keys| {
                filter.exclude_signers(keys)
            }),
        ];
        for (list, add) in lists {
            // `base` already includes one key, so account_include is filled
            // up to the limit, not past it.
            let filled = if list == "account_include" {
                at - 1
            } else {
                at
            };
            assert!(
                one(add(base(), keys(filled))).is_ok(),
                "{list} at the limit"
            );
            assert_eq!(
                one(add(base(), keys(filled + 1))).unwrap_err(),
                FilterError::TooManyAccounts("default".into()),
                "{list} past the limit"
            );
        }
        let signatures = |count: usize| {
            (0..count).map(|index| {
                let mut signature = [0u8; 64];
                signature[..8].copy_from_slice(&index.to_le_bytes());
                Signature::from(signature)
            })
        };
        assert!(one(Filter::new().signatures(signatures(MAX_SIGNATURES_PER_FILTER))).is_ok());
        assert_eq!(
            one(Filter::new().signatures(signatures(MAX_SIGNATURES_PER_FILTER + 1))).unwrap_err(),
            FilterError::TooManySignatures("default".into())
        );
        let region = Region::Harmonic(HarmonicRegion::Ams);
        assert!(
            Filters::new()
                .with("n".repeat(MAX_FILTER_NAME_BYTES), base())
                .into_request(region)
                .is_ok()
        );
        let filters = |count: usize| {
            (0..count).fold(Filters::new(), |filters, index| {
                filters.with(format!("f{index}"), base())
            })
        };
        assert!(filters(MAX_FILTERS).into_request(region).is_ok());
        assert_eq!(
            filters(MAX_FILTERS + 1).into_request(region).unwrap_err(),
            FilterError::TooManyFilters
        );
        let program = key(7);
        let instructions = |count: usize| {
            (0..count).fold(Filters::new(), |filters, index| {
                filters.with(
                    format!("f{index}"),
                    Filter::new().instructions([InstructionFilter::new(program)]),
                )
            })
        };
        assert!(
            instructions(MAX_INSTRUCTION_FILTERS)
                .into_request(region)
                .is_ok()
        );
        assert_eq!(
            instructions(MAX_INSTRUCTION_FILTERS + 1)
                .into_request(region)
                .unwrap_err(),
            FilterError::TooManyInstructionFilters
        );
        let instruction =
            |instruction: InstructionFilter| one(Filter::new().instructions([instruction]));
        let data_end = u32::try_from(MAX_INSTRUCTION_DATA_BYTES).unwrap();
        let memcmp_bytes = u32::try_from(MAX_MEMCMP_BYTES).unwrap();
        for (accepted, what) in [
            (
                (0..MAX_MEMCMPS_PER_INSTRUCTION)
                    .fold(InstructionFilter::new(program), |filter, index| {
                        filter.memcmp(u32::try_from(index).unwrap(), [1])
                    }),
                "the most memcmps",
            ),
            (
                InstructionFilter::new(program).memcmp(0, vec![1; MAX_MEMCMP_BYTES]),
                "the longest memcmp",
            ),
            (
                InstructionFilter::new(program)
                    .memcmp(data_end - memcmp_bytes, vec![1; MAX_MEMCMP_BYTES]),
                "a memcmp ending on the last byte",
            ),
            (
                InstructionFilter::new(program)
                    .memcmp(6, [1, 2, 3])
                    .data_size(9),
                "a memcmp ending on the data size",
            ),
            (
                InstructionFilter::new(program).data_size(data_end),
                "the largest data size",
            ),
            (
                InstructionFilter::new(program).data_size(0),
                "instructions with no data",
            ),
            (
                InstructionFilter::new(program)
                    .memcmp(0, [1])
                    .memcmp(0, [1]),
                "the same memcmp twice",
            ),
        ] {
            assert!(instruction(accepted).is_ok(), "{what}");
        }
        for (refused, error, what) in [
            (
                InstructionFilter::new(program)
                    .memcmp(data_end - memcmp_bytes + 1, vec![1; MAX_MEMCMP_BYTES]),
                FilterError::MemcmpOutOfRange("default".into()),
                "a memcmp ending one past the last byte",
            ),
            (
                InstructionFilter::new(program).memcmp(data_end, [1]),
                FilterError::MemcmpOutOfRange("default".into()),
                "a memcmp starting past the last byte",
            ),
            (
                InstructionFilter::new(program)
                    .memcmp(7, [1, 2, 3])
                    .data_size(9),
                FilterError::MemcmpOutOfRange("default".into()),
                "a memcmp ending one past the data size",
            ),
            (
                InstructionFilter::new(program).memcmp(0, [1]).data_size(0),
                FilterError::MemcmpOutOfRange("default".into()),
                "any memcmp on empty data",
            ),
            (
                InstructionFilter::new(program).data_size(u32::MAX),
                FilterError::DataSizeTooLarge("default".into()),
                "the largest u32 data size",
            ),
        ] {
            assert_eq!(instruction(refused).unwrap_err(), error, "{what}");
        }
    }

    /// Conditions that only narrow a selection are refused on their own and
    /// together; every positive selector is accepted on its own.
    #[test]
    fn a_filter_must_select_something() {
        for (lone, what) in [
            (Filter::new().accounts([key(1)]), "account_include"),
            (Filter::new().require([key(1)]), "account_required"),
            (Filter::new().signers([key(1)]), "signer_include"),
            (
                Filter::new().instructions([InstructionFilter::new(key(1))]),
                "instructions",
            ),
            (
                Filter::new().signatures([Signature::from([1u8; 64])]),
                "signatures",
            ),
        ] {
            assert!(one(lone).is_ok(), "{what} alone is a selection");
        }
        for (narrowing, what) in [
            (
                Filter::new().execution_results([ExecutionResult::Success]),
                "execution_results",
            ),
            (
                Filter::new()
                    .exclude_accounts([key(1)])
                    .exclude_signers([key(2)])
                    .execution_results([ExecutionResult::Success]),
                "every narrowing condition together",
            ),
            (
                Filter::new().accounts([]).signers([]).instructions([]),
                "selectors given empty lists",
            ),
        ] {
            assert_eq!(
                one(narrowing).unwrap_err(),
                FilterError::Empty("default".into()),
                "{what} is not a selection"
            );
        }
    }

    /// In a set, the error names the filter that broke the rule, not the
    /// first one.
    #[test]
    fn the_error_names_the_bad_filter() {
        let region = Region::Harmonic(HarmonicRegion::Ams);
        let program = key(7);
        let good = || Filter::new().accounts([key(1)]);
        for (bad, error) in [
            (Filter::new(), FilterError::Empty("bad".into())),
            (
                Filter::new().instructions([InstructionFilter::new(program).memcmp(0, [])]),
                FilterError::MemcmpLength("bad".into()),
            ),
            (
                good().exclude_signers(keys(MAX_ACCOUNTS_PER_LIST + 1)),
                FilterError::TooManyAccounts("bad".into()),
            ),
        ] {
            let filters = Filters::new()
                .with("a", good())
                .with("bad", bad)
                .with("z", good());
            assert_eq!(filters.into_request(region).unwrap_err(), error);
        }
    }
}
