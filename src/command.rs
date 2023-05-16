use crate::error::{ServerError, ErrorKind};

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
			0 => Command::deserialize_ping(buf),

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

	fn deserialize_ping(buf: &[u8]) -> Result<Self, ServerError> {
		Ok(Command::Ping)
	}
}
