use tokio::net::TcpStream;
use fasthash::murmur3;
use paper_core::stream::{Buffer, StreamReader, StreamError, ErrorKind};
use paper_cache::policy::Policy as CachePolicy;
use crate::server_object::ServerObject;

pub enum Command {
	Ping,
	Version,

	Get(u32),
	Set(u32, ServerObject, Option<u32>),
	Del(u32),

	Clear,

	Resize(u64),
	Policy(&'static CachePolicy),

	Stats,
}

impl Command {
	pub async fn from_stream(stream: &TcpStream) -> Result<Self, StreamError> {
		let reader = StreamReader::new(stream);

		match reader.read_u8().await? {
			0 => Ok(Command::Ping),
			1 => Ok(Command::Version),

			2 => {
				let key = reader.read_buf().await?;

				Ok(Command::Get(hash(&key)))
			},

			3 => {
				let key = reader.read_buf().await?;
				let value = reader.read_buf().await?;

				let ttl = match reader.read_u32().await? {
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
				let key = reader.read_buf().await?;

				Ok(Command::Del(hash(&key)))
			},

			5 => Ok(Command::Clear),

			6 => {
				let size = reader.read_u64().await?;

				Ok(Command::Resize(size))
			},

			7 => {
				let byte = reader.read_u8().await?;

				let policy = match byte {
					0 => &CachePolicy::Lru,
					1 => &CachePolicy::Mru,

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
