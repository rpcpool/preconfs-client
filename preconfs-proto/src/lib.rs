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
}
