use std::net::TcpStream;
use fasthash::murmur3;
use paper_utils::stream::{Buffer, StreamReader, StreamError, ErrorKind};
use paper_cache::policy::Policy as CachePolicy;
use crate::server_object::ServerObject;

pub enum Command {
	Ping,
	Version,

	Get(u32),
	Set(u32, ServerObject, Option<u32>),
	Del(u32),

	Wipe,

	Resize(u64),
	Policy(&'static CachePolicy),

	Stats,
}

impl Command {
	pub fn from_stream(stream: &mut TcpStream) -> Result<Self, StreamError> {
		let mut reader = StreamReader::new(stream);

		match reader.read_u8()? {
			0 => Ok(Command::Ping),
			1 => Ok(Command::Version),

			2 => {
				let key = reader.read_buf()?;

				Ok(Command::Get(hash(&key)))
			},

			3 => {
				let key = reader.read_buf()?;
				let value = reader.read_buf()?;

				let ttl = match reader.read_u32()? {
					0 => None,
					value => Some(value),
				};

				Ok(Command::Set(
					hash(&key),
					ServerObject::new(value),
					ttl
				))
			},

			4 => {
				let key = reader.read_buf()?;

				Ok(Command::Del(hash(&key)))
			},

			5 => Ok(Command::Wipe),

			6 => {
				let size = reader.read_u64()?;

				Ok(Command::Resize(size))
			},

			7 => {
				let byte = reader.read_u8()?;

				let policy = match byte {
					0 => &CachePolicy::Lfu,
					1 => &CachePolicy::Fifo,
					2 => &CachePolicy::Lru,
					3 => &CachePolicy::Mru,

					_ => {
						return Err(StreamError::new(
							ErrorKind::InvalidData,
							"Invalid policy."
						))
					},
				};

				Ok(Command::Policy(policy))
			},

			8 => Ok(Command::Stats),

			_ => Err(StreamError::new(
				ErrorKind::InvalidData,
				"Invalid command."
			))
		}
	}
}

fn hash(data: &Buffer) -> u32 {
	murmur3::hash32(data)
}
