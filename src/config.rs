/*
 * Copyright (c) Kia Shakiba
 *
 * This source code is licensed under the GNU AGPLv3 license found in the
 * LICENSE file in the root directory of this source tree.
 */

use std::{
	env,
	hash::{DefaultHasher, Hash, Hasher},
	include_str,
	path::Path,
	str::FromStr,
	time::Duration,
};

use kwik::file::{FileReader, text::TextReader};
use paper_cache::PaperPolicy;
use parse_size::parse_size;

use crate::error::ServerError;

#[derive(Debug)]
pub struct Config {
	host: String,
	port: u32,

	max_size: u64,
	fast_tier_size: u64,
	policies: Vec<String>,
	policy:   PaperPolicy,

	max_connections: usize,
	auth_token:      Option<u64>,

	/// How often the self-stats report goes to stderr. Command line only.
	stats_interval: Option<Duration>,
}

enum ConfigValue {
	Host(String),
	Port(u32),

	MaxSize(u64),
	FastTierSize(u64),
	PoliciesItem(String),
	Policy(PaperPolicy),

	MaxConnections(usize),
	AuthToken(u64),
}

impl Config {
	pub fn from_file<P>(path: P) -> Result<Self, ServerError>
	where
		P: AsRef<Path>,
	{
		let reader = match TextReader::from_path(path) {
			Ok(reader) => reader,
			Err(_) => return Err(ServerError::InvalidConfig),
		};

		let mut config = init_uninitialized_config();

		let file_iter = reader
			.into_iter()
			.map(|line| line.trim().to_owned())
			.filter(|line| !line.is_empty() && !line.starts_with('#'));

		for line in file_iter {
			Config::parse_line(&mut config, &line)?;
		}

		Ok(config)
	}

	/// Whether this build can serve the configured policy. Checked once the
	/// whole configuration is in, because a build serves one kind of cache: a
	/// tiered build only the hybrid designs, an `all_dram` build only the flat
	/// ones, and a stock config naming `lru` or `auto` is neither.
	pub fn validate(&self) -> Result<(), ServerError> {
		match serves(self.policy) {
			Ok(()) => Ok(()),

			Err(reason) => Err(ServerError::UnservedPolicy(
				self.policy.to_string(),
				reason,
			)),
		}
	}

	pub fn host(&self) -> &str {
		&self.host
	}

	pub fn port(&self) -> u32 {
		self.port
	}

	pub fn max_size(&self) -> u64 {
		self.max_size
	}

	/// The fast (DRAM) tier's byte budget. The remainder of `max_size` is
	/// served from the slow tier. A flat cache has no tier to size, so the
	/// `all_dram` build never reads it (the key still parses, so one config
	/// file serves both builds).
	#[cfg_attr(feature = "all_dram", allow(dead_code))]
	pub fn fast_tier_size(&self) -> u64 {
		self.fast_tier_size
	}

	/// The `policies[]` lines of the file, which nothing reads: a cache's policy
	/// is fixed when it is built, and a tiered one has no `auto` to choose
	/// between several. Kept so a stock config file still loads, and so the
	/// server can say it ignored them.
	pub fn policies(&self) -> &[String] {
		&self.policies
	}

	pub fn policy(&self) -> PaperPolicy {
		self.policy
	}

	pub fn max_connections(&self) -> usize {
		self.max_connections
	}

	pub fn auth_token(&self) -> Option<u64> {
		self.auth_token
	}

	pub fn stats_interval(&self) -> Option<Duration> {
		self.stats_interval
	}

	/// `--stats-interval`: whole seconds, at least one (a zero interval would
	/// print the report as fast as the thread can spin).
	pub fn set_stats_interval(&mut self, seconds: u64) -> Result<(), ServerError> {
		if seconds == 0 {
			return Err(ServerError::InvalidConfigParam("stats_interval"));
		}

		self.stats_interval = Some(Duration::from_secs(seconds));

		Ok(())
	}

	/// `--bind <host>:<port>`: replaces the config's `host` and `port` together.
	pub fn set_bind(&mut self, bind: &str) -> Result<(), ServerError> {
		let Some((host, port)) = bind.rsplit_once(':') else {
			return Err(ServerError::InvalidConfigParam("bind"));
		};

		self.set("host", host)?;
		self.set("port", port)
	}

	/// One config key set to `value`, parsed as the file's lines are: a command
	/// line flag is the config line it stands for, so it takes the same parsing
	/// and wins over the file. (A line of the file cannot hold a `=` in its
	/// value; a flag's can.)
	pub fn set(&mut self, key: &str, value: &str) -> Result<(), ServerError> {
		let token_value = try_parse_env(value).unwrap_or(value.into());

		let config_value = match key {
			"host" => parse_host(&token_value),
			"port" => parse_port(&token_value),

			"max_size" => parse_max_size(&token_value),
			"fast_tier_size" => parse_fast_tier_size(&token_value),
			"policies[]" => parse_policies_item(&token_value),
			"policy" => parse_policy(&token_value),

			"max_connections" => parse_max_connections(&token_value),
			"auth_token" => parse_auth_token(&token_value),

			_ => Err(ServerError::InvalidConfigLine(format!("{key}={value}"))),
		};

		self.apply(config_value?);

		Ok(())
	}

	fn parse_line(config: &mut Config, line: &str) -> Result<(), ServerError> {
		let tokens: Vec<&str> = line.split('=').collect();

		if tokens.len() != 2 {
			return Err(ServerError::InvalidConfigLine(line.into()));
		}

		config.set(tokens[0], tokens[1])
	}

	fn apply(&mut self, value: ConfigValue) {
		match value {
			ConfigValue::Host(host) => self.host = host,
			ConfigValue::Port(port) => self.port = port,

			ConfigValue::MaxSize(max_size) => self.max_size = max_size,
			ConfigValue::FastTierSize(size) => self.fast_tier_size = size,
			ConfigValue::PoliciesItem(policy) => self.policies.push(policy),
			ConfigValue::Policy(policy) => self.policy = policy,

			ConfigValue::MaxConnections(max_connections) => {
				self.max_connections = max_connections
			},
			ConfigValue::AuthToken(token) => self.auth_token = Some(token),
		}
	}
}

impl Default for Config {
	fn default() -> Self {
		let default_config_data = include_str!("../default.pconf");
		let mut config = init_uninitialized_config();

		let line_iter = default_config_data
			.split('\n')
			.map(|line| line.trim().to_owned())
			.filter(|line| !line.is_empty() && !line.starts_with('#'));

		for line in line_iter {
			Config::parse_line(&mut config, &line)
				.expect("An error occured when parsing default config");
		}

		// default.pconf names the tiered build's design. The flat build has no
		// hybrid design to default to, so it takes the flat counterpart.
		#[cfg(feature = "all_dram")]
		{
			config.policy = PaperPolicy::LruCompact;
		}

		config
	}
}

/// Why this build cannot serve `policy`, if it cannot.
#[cfg(feature = "tiered")]
fn serves(policy: PaperPolicy) -> Result<(), &'static str> {
	if !policy.is_hybrid() {
		return Err(
			"not a hybrid design; this build serves the tiered cache, and a \
			 flat design needs the all_dram build",
		);
	}

	// `PaperCache::new` answers InvalidPolicy for it: the size-split design
	// takes three sizing scalars and has a constructor of its own.
	if matches!(policy, PaperPolicy::LruSizedCompactHybrid) {
		return Err(
			"the size-split design has its own constructor, which this \
			 server does not use",
		);
	}

	Ok(())
}

#[cfg(feature = "all_dram")]
fn serves(policy: PaperPolicy) -> Result<(), &'static str> {
	if policy.is_hybrid() {
		return Err(
			"a hybrid design; this all_dram build has no slow tier to place \
			 anything in",
		);
	}

	Ok(())
}

fn init_uninitialized_config() -> Config {
	Config {
		host: String::new(),
		port: 0,

		max_size: 0,
		fast_tier_size: 0,
		policies: Vec::new(),
		policy:   PaperPolicy::LruCompactHybrid,

		max_connections: 0,
		auth_token:      None,

		stats_interval: None,
	}
}

fn try_parse_env(value: &str) -> Option<String> {
	let value = value.trim();

	match value.starts_with('$') {
		true => env::var(&value[1..]).ok(),
		false => None,
	}
}

fn parse_host(value: &str) -> Result<ConfigValue, ServerError> {
	if value.is_empty() {
		return Err(ServerError::InvalidConfigParam("host"));
	}

	Ok(ConfigValue::Host(value.to_owned()))
}

fn parse_port(value: &str) -> Result<ConfigValue, ServerError> {
	match value.parse::<u32>() {
		Ok(value) => Ok(ConfigValue::Port(value)),
		Err(_) => Err(ServerError::InvalidConfigParam("port")),
	}
}

fn parse_fast_tier_size(value: &str) -> Result<ConfigValue, ServerError> {
	match parse_size(value) {
		Ok(0) | Err(_) => Err(ServerError::InvalidConfigParam("fast_tier_size")),
		Ok(value) => Ok(ConfigValue::FastTierSize(value)),
	}
}

fn parse_max_size(value: &str) -> Result<ConfigValue, ServerError> {
	match parse_size(value) {
		Ok(0) | Err(_) => Err(ServerError::InvalidConfigParam("max_size")),
		Ok(value) => Ok(ConfigValue::MaxSize(value)),
	}
}

/// Not validated: the names a stock config lists here (`lru`, `arc`,
/// `s3-fifo-0.1`, ...) are not all names this cache still knows, and nothing
/// reads them.
fn parse_policies_item(value: &str) -> Result<ConfigValue, ServerError> {
	Ok(ConfigValue::PoliciesItem(value.to_owned()))
}

fn parse_policy(value: &str) -> Result<ConfigValue, ServerError> {
	match PaperPolicy::from_str(value) {
		Ok(policy) => Ok(ConfigValue::Policy(policy)),
		Err(_) => Err(ServerError::InvalidConfigPolicy(value.into())),
	}
}

fn parse_max_connections(value: &str) -> Result<ConfigValue, ServerError> {
	match value.parse::<usize>() {
		Ok(0) | Err(_) => Err(ServerError::InvalidConfigParam("max_connections")),
		Ok(value) => Ok(ConfigValue::MaxConnections(value)),
	}
}

fn parse_auth_token(value: &str) -> Result<ConfigValue, ServerError> {
	if value.is_empty() {
		return Err(ServerError::InvalidConfigParam("auth_token"));
	}

	let mut s = DefaultHasher::new();
	value.hash(&mut s);

	Ok(ConfigValue::AuthToken(s.finish()))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_default_config_is_servable_by_this_build() {
		let config = Config::default();

		assert!(config.validate().is_ok());
		assert_eq!(config.max_connections(), 50);
		assert_eq!(config.auth_token(), None);

		// A default start must not warn that it ignores a `policies[]` list.
		assert!(config.policies().is_empty());
	}

	#[test]
	fn a_flag_replaces_what_the_defaults_set() {
		let mut config = Config::default();

		config.set_bind("127.0.0.1:3700").unwrap();
		config.set("max_size", "16106127360").unwrap();
		config.set("fast_tier_size", "5GiB").unwrap();
		config.set("auth_token", "a=b==").unwrap();

		assert_eq!(config.host(), "127.0.0.1");
		assert_eq!(config.port(), 3700);
		assert_eq!(config.max_size(), 16_106_127_360);
		assert_eq!(config.fast_tier_size(), 5 << 30);
		assert!(config.auth_token().is_some());

		// A bracketed IPv6 address keeps its colons: the port is after the last.
		config.set_bind("[::1]:3146").unwrap();
		assert_eq!((config.host(), config.port()), ("[::1]", 3146));
	}

	#[test]
	fn a_bad_flag_is_refused_and_leaves_the_setting_alone() {
		let mut config = Config::default();

		assert!(config.set_bind("no-port").is_err());
		assert!(config.set_bind("host:not-a-port").is_err());
		assert!(config.set("max_size", "0").is_err());
		assert!(config.set("max_size", "lots").is_err());
		assert!(config.set("policy", "no-such-design").is_err());
		assert!(config.set("no_such_key", "1").is_err());
		assert!(config.set_stats_interval(0).is_err());

		assert_eq!(config.max_size(), 2 << 30);
		assert_eq!(config.stats_interval(), None);
	}

	#[test]
	fn policies_lines_are_accepted_whatever_they_name() {
		let mut config = Config::default();

		// Names a stock config lists, most of which this cache no longer knows.
		for name in ["lru", "arc", "s3-fifo-0.1", "2q-0.2-0.5", "auto"] {
			Config::parse_line(&mut config, &format!("policies[]={name}")).unwrap();
		}

		assert_eq!(config.policies().len(), 5);
		assert!(config.validate().is_ok());
	}

	#[cfg(feature = "tiered")]
	#[test]
	fn a_tiered_build_serves_only_the_hybrid_designs() {
		let mut config = Config::default();

		for policy in ["lru-compact", "arc", "s3-fifo-compact-0.1"] {
			config.set("policy", policy).unwrap();
			assert!(config.validate().is_err(), "{policy} is flat");
		}

		config.set("policy", "lru-sized-compact-hybrid").unwrap();
		assert!(config.validate().is_err(), "the size-split design needs its own constructor");

		for policy in [
			"lru-compact-hybrid",
			"fifo-compact-hybrid",
			"s3-fifo-faithful-compact-hybrid-0.1",
			"2q-full-fast-admission-compact-hybrid-0.2-0.5",
		] {
			config.set("policy", policy).unwrap();
			assert!(config.validate().is_ok(), "{policy} is a hybrid design");
		}
	}

	#[cfg(feature = "all_dram")]
	#[test]
	fn a_flat_build_serves_only_the_flat_designs() {
		let mut config = Config::default();

		// The default is the flat counterpart of the tiered build's.
		assert_eq!(config.policy(), PaperPolicy::LruCompact);

		config.set("policy", "lru-compact-hybrid").unwrap();
		assert!(config.validate().is_err());
	}
}
