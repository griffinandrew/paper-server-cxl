use kwik::text_reader::{FileReader, TextReader};
use paper_cache::policy::Policy as CachePolicy;
use crate::server_error::{ServerError, ErrorKind};

pub struct Config {
	max_size: u64,
	policy: CachePolicy,
}

enum ConfigValue {
	MaxSize(u64),
	Policy(CachePolicy),
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
			max_size: 0,
			policy: CachePolicy::Lru,
		};

		while let Some(line) = reader.read_line() {
			if line.len() == 0 {
				continue;
			}

			if let Err(err) = Config::parse_line(&mut config, &line) {
				return Err(err);
			}
		}

		Ok(config)
	}

	pub fn get_max_size(&self) -> &u64 {
		&self.max_size
	}

	pub fn get_policy(&self) -> &CachePolicy {
		&self.policy
	}

	fn parse_line(config: &mut Config, line: &String) -> Result<(), ServerError> {
		let tokens: Vec<&str> = line.split("=").collect();

		if tokens.len() != 2 {
			return Err(ServerError::new(
				ErrorKind::InvalidConfig,
				&format!("Invalid config line <{}>", line)
			));
		}

		let config_value = match tokens[0] {
			"max_size" => parse_max_size(&tokens[1]),
			"policy" => parse_policy(&tokens[1]),

			_ => Err(ServerError::new(
				ErrorKind::InvalidConfig,
				&format!("Invalid config line <{}>", line)
			)),
		};

		match config_value {
			Ok(value) => {
				match value {
					ConfigValue::MaxSize(max_size) => config.max_size = max_size,
					ConfigValue::Policy(policy) => config.policy = policy,
				}
			},

			Err(err) => {
				return Err(err);
			},
		}

		Ok(())
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

fn parse_policy(value: &str) -> Result<ConfigValue, ServerError> {
	match value {
		"lru" => Ok(ConfigValue::Policy(CachePolicy::Lru)),
		"mru" => Ok(ConfigValue::Policy(CachePolicy::Mru)),

		_ => Err(ServerError::new(
			ErrorKind::InvalidConfig,
			"Invalid policy config."
		)),
	}
}
