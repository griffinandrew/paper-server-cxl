use thiserror::Error;

#[derive(Debug, PartialEq, Error)]
pub enum ServerError {
	#[error("Could not establish a connection.")]
	InvalidAddress,

	#[error("Could not establish a connection.")]
	InvalidConnection,

	#[error("The maximum number of connections was exceeded.")]
	MaxConnectionsExceeded,

	#[error("{0}")]
	InvalidCommand(String),

	#[error("Invalid response.")]
	InvalidResponse,

	#[error("Disconnected from client.")]
	Disconnected,

	#[error("Could not open config file.")]
	InvalidConfig,

	#[error("Invalid config line <{0}>.")]
	InvalidConfigLine(String),

	#[error("Invalid {0} config.")]
	InvalidConfigParam(&'static str),

	#[error("Invalid policy <{0}> in config.")]
	InvalidConfigPolicy(String),
}
