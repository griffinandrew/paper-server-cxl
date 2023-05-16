use crate::server_error::{ServerError, ErrorKind};

pub enum Command {
	Ping,

	Get,
	Set,
	Del,

	Resize,
	Policy,
}

impl Command {
	pub fn deserialize(buf: &[u8]) -> Result<Self, ServerError> {
		match buf[0] {
			0 => Ok(Command::Ping),

			1 => Ok(Command::Get),
			2 => Ok(Command::Set),
			3 => Ok(Command::Del),

			4 => Ok(Command::Resize),
			5 => Ok(Command::Policy),

			_ => Err(ServerError::new(
				ErrorKind::InvalidCommand,
				"Invalid command."
			))
		}
	}
}
