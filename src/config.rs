use parse_size::parse_size;
use kwik::text_reader::{FileReader, TextReader};
use paper_cache::policy::Policy as CachePolicy;
use crate::server_error::ServerError;

pub struct Config {
	host: String,
	port: u32,

	max_size: u64,
	policies: Vec<CachePolicy>,

	max_connections: usize,
}

enum ConfigValue {
	Host(String),
	Port(u32),

	MaxSize(u64),
	Policies(Vec<CachePolicy>),

	MaxConnections(usize),
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

	pub fn policies(&self) -> &[CachePolicy] {
		&self.policies
	}

	pub fn max_connections(&self) -> usize {
		self.max_connections
	}

	fn parse_line(config: &mut Config, line: &str) -> Result<(), ServerError> {
		let tokens: Vec<&str> = line.split('=').collect();

		if tokens.len() != 2 {
			return Err(ServerError::InvalidConfigLine(line.into()));
		}

		let config_value = match tokens[0] {
			"host" => parse_host(tokens[1]),
			"port" => parse_port(tokens[1]),

			"max_size" => parse_max_size(tokens[1]),
			"policies" => parse_policies(tokens[1]),

			"max_connections" => parse_max_connections(tokens[1]),

			_ => Err(ServerError::InvalidConfigLine(line.into())),
		};

		match config_value {
			Ok(value) => match value {
				ConfigValue::Host(host) => config.host = host,
				ConfigValue::Port(port) => config.port = port,

				ConfigValue::MaxSize(max_size) => config.max_size = max_size,
				ConfigValue::Policies(policies) => config.policies = policies,

				ConfigValue::MaxConnections(max_connections) => config.max_connections = max_connections,
			},

			Err(err) => return Err(err),
		}

		Ok(())
	}
}

fn parse_host(value: &str) -> Result<ConfigValue, ServerError> {
	if value.is_empty() {
		return Err(ServerError::InvalidConfig);
	}

	Ok(ConfigValue::Host(value.to_owned()))
}

fn parse_port(value: &str) -> Result<ConfigValue, ServerError> {
	match value.parse::<u32>() {
		Ok(value) => Ok(ConfigValue::Port(value)),
		Err(_) => Err(ServerError::InvalidConfig),
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

	let mut policies = Vec::<CachePolicy>::new();

	for token in tokens {
		match token {
			"lfu" => policies.push(CachePolicy::Lfu),
			"fifo" => policies.push(CachePolicy::Fifo),
			"lru" => policies.push(CachePolicy::Lru),
			"mru" => policies.push(CachePolicy::Mru),
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
