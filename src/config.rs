use std::env;
use parse_size::parse_size;

use kwik::file::{
	FileReader,
	text::TextReader,
};

use paper_cache::policy::PaperPolicy;
use crate::server_error::ServerError;

#[derive(Debug)]
pub struct Config {
	host: String,
	port: u32,

	max_size: u64,
	policies: Vec<PaperPolicy>,

	max_connections: usize,
	auth: Option<String>,
}

enum ConfigValue {
	Host(String),
	Port(u32),

	MaxSize(u64),
	Policies(Vec<PaperPolicy>),

	MaxConnections(usize),
	Auth(String),
}

impl Config {
	pub fn from_file(path: &str) -> Result<Self, ServerError> {
		let reader = match TextReader::new(path) {
			Ok(reader) => reader,
			Err(_) => return Err(ServerError::InvalidConfig),
		};

		let mut config = Config {
			host: String::new(),
			port: 0,

			max_size: 0,
			policies: Vec::new(),

			max_connections: 0,
			auth: None,
		};

		let file_iter = reader
			.into_iter()
			.map(|line| line.trim().to_owned())
			.filter(|line| !line.is_empty() && !line.starts_with('#'));

		for line in file_iter {
			Config::parse_line(&mut config, &line)?;
		}

		Ok(config)
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

	pub fn policies(&self) -> &[PaperPolicy] {
		&self.policies
	}

	pub fn max_connections(&self) -> usize {
		self.max_connections
	}

	pub fn auth(&self) -> Option<&str> {
		self.auth.as_deref()
	}

	fn parse_line(config: &mut Config, line: &str) -> Result<(), ServerError> {
		let tokens: Vec<&str> = line.split('=').collect();

		if tokens.len() != 2 {
			return Err(ServerError::InvalidConfigLine(line.into()));
		}

		let token_value = try_parse_env(tokens[1])
			.unwrap_or(tokens[1].into());

		let config_value = match tokens[0] {
			"host" => parse_host(&token_value),
			"port" => parse_port(&token_value),

			"max_size" => parse_max_size(&token_value),
			"policies" => parse_policies(&token_value),

			"max_connections" => parse_max_connections(&token_value),
			"auth" => parse_auth(&token_value),

			_ => Err(ServerError::InvalidConfigLine(line.into())),
		};

		match config_value {
			Ok(value) => match value {
				ConfigValue::Host(host) => config.host = host,
				ConfigValue::Port(port) => config.port = port,

				ConfigValue::MaxSize(max_size) => config.max_size = max_size,
				ConfigValue::Policies(policies) => config.policies = policies,

				ConfigValue::MaxConnections(max_connections) => config.max_connections = max_connections,
				ConfigValue::Auth(token) => config.auth = Some(token),
			},

			Err(err) => return Err(err),
		}

		Ok(())
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

fn parse_max_size(value: &str) -> Result<ConfigValue, ServerError> {
	match parse_size(value) {
		Ok(0) | Err(_) => Err(ServerError::InvalidConfigParam("max_size")),
		Ok(value) => Ok(ConfigValue::MaxSize(value)),
	}
}

fn parse_policies(value: &str) -> Result<ConfigValue, ServerError> {
	let tokens: Vec<&str> = value.split('|').collect();

	if tokens.is_empty() {
		return Err(ServerError::InvalidConfigParam("policies"));
	}

	let mut policies = Vec::<PaperPolicy>::new();

	for token in tokens {
		match token {
			"lfu" => policies.push(PaperPolicy::Lfu),
			"fifo" => policies.push(PaperPolicy::Fifo),
			"lru" => policies.push(PaperPolicy::Lru),
			"mru" => policies.push(PaperPolicy::Mru),
			_ => return Err(ServerError::InvalidConfigPolicy(token.into())),
		}
	}

	Ok(ConfigValue::Policies(policies))
}

fn parse_max_connections(value: &str) -> Result<ConfigValue, ServerError> {
	match value.parse::<usize>() {
		Ok(0) | Err(_) => Err(ServerError::InvalidConfigParam("max_connections")),
		Ok(value) => Ok(ConfigValue::MaxConnections(value)),
	}
}

fn parse_auth(value: &str) -> Result<ConfigValue, ServerError> {
	if value.is_empty() {
		return Err(ServerError::InvalidConfigParam("auth"));
	}

	Ok(ConfigValue::Auth(value.to_owned()))
}
