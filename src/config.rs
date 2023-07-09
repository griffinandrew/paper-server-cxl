use kwik::text_reader::{FileReader, TextReader};
use paper_cache::policy::Policy as CachePolicy;
use crate::server_error::{ServerError, ErrorKind};

pub struct Config {
	host: String,
	port: u32,

	max_size: u64,
	policies: Vec<&'static CachePolicy>,
}

enum ConfigValue {
	Host(String),
	Port(u32),

	MaxSize(u64),
	Policies(Vec<&'static CachePolicy>),
}

impl Config {
	pub fn from_file(path: &str) -> Result<Self, ServerError> {
		let mut reader = match TextReader::new(path) {
			Ok(reader) => reader,

			Err(_) => {
				return Err(ServerError::new(
					ErrorKind::InvalidConfig,
					"Could not open config file."
				));
			},
		};

		let mut config = Config {
			host: String::new(),
			port: 0,

			max_size: 0,
			policies: vec![],
		};

		while let Some(line) = reader.read_line() {
			let trimmed_line = line.trim();

			if trimmed_line.is_empty() || trimmed_line.starts_with('#') {
				continue;
			}

			Config::parse_line(&mut config, line)?;
		}

		Ok(config)
	}

	pub fn host(&self) -> &str {
		&self.host
	}

	pub fn port(&self) -> &u32 {
		&self.port
	}

	pub fn max_size(&self) -> &u64 {
		&self.max_size
	}

	pub fn policies(&self) -> &Vec<&'static CachePolicy> {
		&self.policies
	}

	fn parse_line(config: &mut Config, line: &String) -> Result<(), ServerError> {
		let tokens: Vec<&str> = line.split('=').collect();

		if tokens.len() != 2 {
			return Err(ServerError::new(
				ErrorKind::InvalidConfig,
				&format!("Invalid config line <{}>", line)
			));
		}

		let config_value = match tokens[0] {
			"host" => Ok(ConfigValue::Host(tokens[1].to_string())),
			"port" => parse_port(tokens[1]),

			"max_size" => parse_max_size(tokens[1]),
			"policies" => parse_policies(tokens[1]),

			_ => Err(ServerError::new(
				ErrorKind::InvalidConfig,
				&format!("Invalid config line <{}>", line)
			)),
		};

		match config_value {
			Ok(value) => {
				match value {
					ConfigValue::Host(host) => config.host = host,
					ConfigValue::Port(port) => config.port = port,

					ConfigValue::MaxSize(max_size) => config.max_size = max_size,
					ConfigValue::Policies(policies) => config.policies = policies,
				}
			},

			Err(err) => {
				return Err(err);
			},
		}

		Ok(())
	}
}

fn parse_port(value: &str) -> Result<ConfigValue, ServerError> {
	match value.parse::<u32>() {
		Ok(value) => Ok(ConfigValue::Port(value)),

		Err(_) => Err(ServerError::new(
			ErrorKind::InvalidConfig,
			"Invalid port config."
		)),
	}
}

fn parse_max_size(value: &str) -> Result<ConfigValue, ServerError> {
	match value.parse::<u64>() {
		Ok(value) => Ok(ConfigValue::MaxSize(value)),

		Err(_) => Err(ServerError::new(
			ErrorKind::InvalidConfig,
			"Invalid max_size config."
		)),
	}
}

fn parse_policies(value: &str) -> Result<ConfigValue, ServerError> {
	let tokens: Vec<&str> = value.split('|').collect();

	if tokens.is_empty() {
		return Err(ServerError::new(
			ErrorKind::InvalidConfig,
			"Invalid policies config."
		));
	}

	let mut policies = Vec::<&CachePolicy>::new();

	for token in tokens {
		match token {
			"lfu" => policies.push(&CachePolicy::Lfu),
			"fifo" => policies.push(&CachePolicy::Fifo),
			"lru" => policies.push(&CachePolicy::Lru),
			"mru" => policies.push(&CachePolicy::Mru),

			_ => {
				return Err(ServerError::new(
					ErrorKind::InvalidConfig,
					&format!("Invalid policy <{}> in config.", token)
				));
			},
		}
	}

	Ok(ConfigValue::Policies(policies))
}
