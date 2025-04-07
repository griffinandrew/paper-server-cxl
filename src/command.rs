use std::net::TcpStream;

use paper_utils::{
	stream::{Buffer, StreamReader, StreamError},
	command::CommandByte,
};

pub enum Command {
	Ping,
	Version,

	Auth(Buffer),

	Get(Buffer),
	Set(Buffer, Buffer, Option<u32>),
	Del(Buffer),

	Has(Buffer),
	Peek(Buffer),
	Ttl(Buffer, Option<u32>),
	Size(Buffer),

	Wipe,

	Resize(u64),
	Policy(String),

	Stats,
}

impl Command {
	pub fn from_stream(stream: &mut TcpStream) -> Result<Self, StreamError> {
		let mut reader = StreamReader::new(stream);

		match reader.read_u8()? {
			CommandByte::PING => Ok(Command::Ping),
			CommandByte::VERSION => Ok(Command::Version),

			CommandByte::AUTH => {
				let token = reader.read_buf()?;
				Ok(Command::Auth(token))
			},

			CommandByte::GET => {
				let key = reader.read_buf()?;
				Ok(Command::Get(key))
			},

			CommandByte::SET => {
				let key = reader.read_buf()?;
				let value = reader.read_buf()?;

				let ttl = match reader.read_u32()? {
					0 => None,
					value => Some(value),
				};

				Ok(Command::Set(key, value, ttl))
			},

			CommandByte::DEL => {
				let key = reader.read_buf()?;
				Ok(Command::Del(key))
			},

			CommandByte::HAS => {
				let key = reader.read_buf()?;
				Ok(Command::Has(key))
			},

			CommandByte::PEEK => {
				let key = reader.read_buf()?;
				Ok(Command::Peek(key))
			},

			CommandByte::TTL => {
				let key = reader.read_buf()?;

				let ttl = match reader.read_u32()? {
					0 => None,
					value => Some(value),
				};

				Ok(Command::Ttl(key, ttl))
			},

			CommandByte::SIZE => {
				let key = reader.read_buf()?;
				Ok(Command::Size(key))
			},

			CommandByte::WIPE => Ok(Command::Wipe),

			CommandByte::RESIZE => {
				let size = reader.read_u64()?;
				Ok(Command::Resize(size))
			},

			CommandByte::POLICY => {
				let policy_str = reader.read_string()?;
				Ok(Command::Policy(policy_str))
			},

			CommandByte::STATS => Ok(Command::Stats),

			_ => Err(StreamError::InvalidData),
		}
	}
}
