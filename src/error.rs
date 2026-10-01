/*
 * Copyright (c) Kia Shakiba
 *
 * This source code is licensed under the GNU AGPLv3 license found in the
 * LICENSE file in the root directory of this source tree.
 */

use paper_cache::CacheError;
use paper_utils::sheet::{Sheet, SheetBuilder};
use thiserror::Error;

#[derive(Debug, PartialEq, Error)]
pub enum ServerError {
	#[error(transparent)]
	CacheError(#[from] CacheError),

	#[error("internal error")]
	Internal,

	#[error("could not establish a connection")]
	InvalidAddress,

	#[error("could not establish a connection")]
	InvalidConnection,

	#[error("the maximum number of connections was exceeded")]
	MaxConnectionsExceeded,

	#[error("{0}")]
	InvalidCommand(String),

	#[error("invalid response")]
	InvalidResponse,

	#[error("disconnected from client")]
	Disconnected,

	#[error("could not open config file")]
	InvalidConfig,

	#[error("invalid config line <{0}>")]
	InvalidConfigLine(String),

	#[error("invalid {0} config")]
	InvalidConfigParam(&'static str),

	#[error("invalid policy <{0}> in config")]
	InvalidConfigPolicy(String),

	#[error("policy <{0}> cannot be served by this build: {1}")]
	UnservedPolicy(String, &'static str),

	#[error("unauthorized")]
	Unauthorized,
}

impl ServerError {
	pub fn to_sheet(&self) -> Sheet {
		if let ServerError::CacheError(err) = self {
			return SheetBuilder::new()
				.write_bool(false)
				.write_u8(get_error_code(self))
				.write_u8(get_cache_error_code(err))
				.into_sheet();
		}

		SheetBuilder::new()
			.write_bool(false)
			.write_u8(get_error_code(self))
			.into_sheet()
	}
}

fn get_error_code(error: &ServerError) -> u8 {
	match error {
		ServerError::CacheError(_) => 0,

		ServerError::Internal
		| ServerError::InvalidAddress
		| ServerError::InvalidConnection
		| ServerError::InvalidCommand(_)
		| ServerError::InvalidResponse
		| ServerError::Disconnected
		| ServerError::InvalidConfig
		| ServerError::InvalidConfigLine(_)
		| ServerError::InvalidConfigParam(_)
		| ServerError::InvalidConfigPolicy(_)
		| ServerError::UnservedPolicy(..) => 1,

		ServerError::MaxConnectionsExceeded => 2,
		ServerError::Unauthorized => 3,
	}
}

/// The cache error codes: the second byte of a reply that starts `[false][0]`.
///
/// Every variant is spelled out, so a variant the cache adds does not compile
/// until it is given a code here. This used to end in `_ => 0`, which reports
/// "no code" for anything it does not list, and a stock client reads a 0 as an
/// internal error: a stalled fast tier would have looked like a bug.
///
/// 1 to 6 are upstream's. The rest are this fork's, and a stock client decodes
/// each of them as `PaperCacheError::Internal` (its `from_code` ends in
/// `_ => Internal`); the frame is the same shape, so it stays in step and
/// reads one more error than it knows by name.
fn get_cache_error_code(error: &CacheError) -> u8 {
	match error {
		CacheError::KeyNotFound => 1,

		CacheError::ZeroValueSize => 2,
		CacheError::ExceedingValueSize => 3,

		CacheError::ZeroCacheSize => 4,

		CacheError::UnconfiguredPolicy => 5,
		CacheError::InvalidPolicy => 6,

		// A merged build refusing a policy whose eviction order it does not
		// implement. The cache is built before any connection is accepted, so
		// in practice this is a refusal to START, said on the console; it is
		// given a code so that it could never be mistaken for "no code".
		CacheError::PolicyNotImplemented(..) => 7,

		// The set path's admission refusals (S9). 8: the fast tier stayed full
		// for as long as the set would wait (the byte gate's stall window, or
		// `set_timeout`). 9: a new key's metadata would not fit the fast tier
		// (or `EvictToFit` found no room in time). Nothing was allocated or
		// inserted; retrying later may succeed.
		CacheError::FastTierStalled => 8,
		CacheError::MetadataOverflow => 9,

		// What a stock client already reads as an internal error, listed so
		// that adding a variant is a decision and not a silent 0.
		CacheError::Internal
		| CacheError::EmptyPolicies
		| CacheError::DuplicatePolicies
		| CacheError::AllocationFailed
		| CacheError::InvalidFastTierSize
		| CacheError::InvalidGateConfig => 0,
	}
}

#[cfg(test)]
mod tests {
	use paper_cache::PaperPolicy;

	use super::*;

	/// The reply a client reads for a cache error: `[?][0][code]`.
	fn frame(error: CacheError) -> Vec<u8> {
		ServerError::CacheError(error).to_sheet().serialize().to_vec()
	}

	#[test]
	fn the_admission_refusals_have_codes_of_their_own() {
		assert_eq!(frame(CacheError::FastTierStalled), [b'?', 0, 8]);
		assert_eq!(frame(CacheError::MetadataOverflow), [b'?', 0, 9]);
	}

	#[test]
	fn upstreams_codes_are_unchanged() {
		assert_eq!(frame(CacheError::KeyNotFound), [b'?', 0, 1]);
		assert_eq!(frame(CacheError::ZeroValueSize), [b'?', 0, 2]);
		assert_eq!(frame(CacheError::ExceedingValueSize), [b'?', 0, 3]);
		assert_eq!(frame(CacheError::ZeroCacheSize), [b'?', 0, 4]);
		assert_eq!(frame(CacheError::UnconfiguredPolicy), [b'?', 0, 5]);
		assert_eq!(frame(CacheError::InvalidPolicy), [b'?', 0, 6]);
		assert_eq!(frame(CacheError::PolicyNotImplemented(PaperPolicy::LruCompactHybrid)), [b'?', 0, 7]);
	}

	/// No two refusals share a code, and the ones a stock client cannot name
	/// are the only ones that read as 0, so a 0 is never a refusal.
	#[test]
	fn only_internal_errors_read_as_no_code() {
		for error in [
			CacheError::Internal,
			CacheError::EmptyPolicies,
			CacheError::DuplicatePolicies,
			CacheError::AllocationFailed,
			CacheError::InvalidFastTierSize,
			CacheError::InvalidGateConfig,
		] {
			assert_eq!(get_cache_error_code(&error), 0, "{error}");
		}

		let codes: Vec<u8> = [
			CacheError::KeyNotFound,
			CacheError::ZeroValueSize,
			CacheError::ExceedingValueSize,
			CacheError::ZeroCacheSize,
			CacheError::UnconfiguredPolicy,
			CacheError::InvalidPolicy,
			CacheError::PolicyNotImplemented(PaperPolicy::LruCompactHybrid),
			CacheError::FastTierStalled,
			CacheError::MetadataOverflow,
		]
		.iter()
		.map(get_cache_error_code)
		.collect();

		assert_eq!(codes, [1, 2, 3, 4, 5, 6, 7, 8, 9]);
	}

	#[test]
	fn a_server_error_is_one_code_byte() {
		assert_eq!(ServerError::Unauthorized.to_sheet().serialize(), [b'?', 3]);
		assert_eq!(ServerError::MaxConnectionsExceeded.to_sheet().serialize(), [b'?', 2]);
	}
}
