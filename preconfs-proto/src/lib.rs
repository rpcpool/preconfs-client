//! Triton Preconfs API: the messages and gRPC clients generated from
//! `proto/preconfs.proto` (services `preconfs.Harmonic` and `preconfs.BAM`).
//!
//! Most programs want `triton-preconfs-client`, which wraps these types with
//! connection handling, typed events and filter validation. This crate is
//! for tooling that needs the raw schema or the generated clients.

use std::{fmt, str::FromStr};

/// The generated messages and gRPC clients.
pub mod preconfs {
    // Generated code does not follow the workspace lints.
    #![allow(clippy::clone_on_ref_ptr, clippy::missing_const_for_fn)]
    #![allow(missing_docs)]
    tonic::include_proto!("preconfs");
}

/// The schema this crate was built from, for tooling in other languages.
pub const PROTO_SOURCE: &str = include_str!("../proto/preconfs.proto");

pub use {prost, tonic};

impl preconfs::HarmonicTransaction {
    /// The builder's outcome; `None` when the server did not know it. Use
    /// this rather than the generated `result()`, which returns the zero
    /// value, `Success`, for an unset or unknown outcome.
    pub fn execution_result(&self) -> Option<preconfs::ExecutionResult> {
        self.result
            .and_then(|value| preconfs::ExecutionResult::try_from(value).ok())
    }
}

impl preconfs::BamTransaction {
    /// The outcome the leader reported to the BAM node, success or execution
    /// failure; `None` when the node did not report one. Use this rather
    /// than the generated `result()`, which returns `Success` for an unset or
    /// unknown outcome.
    pub fn execution_result(&self) -> Option<preconfs::ExecutionResult> {
        self.result
            .and_then(|value| preconfs::ExecutionResult::try_from(value).ok())
    }
}

/// A name that is not an [`preconfs::ExecutionResult`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownExecutionResult(pub String);

impl fmt::Display for UnknownExecutionResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown execution result {:?}; expected success, execution_failure or fees_only",
            self.0
        )
    }
}

impl std::error::Error for UnknownExecutionResult {}

/// Parses the short lowercase names (`success`, `execution_failure`,
/// `fees_only`) as well as the proto names (`EXECUTION_RESULT_SUCCESS`).
impl FromStr for preconfs::ExecutionResult {
    type Err = UnknownExecutionResult;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        let short = match name.to_ascii_lowercase().as_str() {
            "success" => Some(Self::Success),
            "execution_failure" => Some(Self::ExecutionFailure),
            "fees_only" => Some(Self::FeesOnly),
            _ => None,
        };
        short
            .or_else(|| Self::from_str_name(name))
            .ok_or_else(|| UnknownExecutionResult(name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use {super::*, preconfs::ExecutionResult};

    #[test]
    fn execution_results_parse_short_and_proto_names() {
        assert_eq!(
            "fees_only".parse::<ExecutionResult>().unwrap(),
            ExecutionResult::FeesOnly
        );
        assert_eq!(
            "EXECUTION_RESULT_SUCCESS"
                .parse::<ExecutionResult>()
                .unwrap(),
            ExecutionResult::Success
        );
        assert_eq!(
            "Success".parse::<ExecutionResult>().unwrap(),
            ExecutionResult::Success
        );
        assert!("landed".parse::<ExecutionResult>().is_err());
    }

    /// Unset and unknown outcomes are `None`, never the zero value.
    #[test]
    fn execution_result_is_checked() {
        let harmonic = |result| preconfs::HarmonicTransaction {
            result,
            ..Default::default()
        };
        let bam = |result| preconfs::BamTransaction {
            result,
            ..Default::default()
        };
        for (value, expected) in [
            (None, None),
            (Some(0), Some(ExecutionResult::Success)),
            (Some(1), Some(ExecutionResult::ExecutionFailure)),
            (Some(2), Some(ExecutionResult::FeesOnly)),
            (Some(3), None),
            (Some(-1), None),
        ] {
            assert_eq!(harmonic(value).execution_result(), expected, "{value:?}");
            assert_eq!(bam(value).execution_result(), expected, "{value:?}");
        }
        // The generated accessor is the trap the checked one avoids.
        assert_eq!(harmonic(Some(3)).result(), ExecutionResult::Success);
    }
}
