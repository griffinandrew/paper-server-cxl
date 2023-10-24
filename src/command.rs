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
	Peek(u32),

	Wipe,

	Resize(u64),
	Policy(CachePolicy),

	Stats,
}

struct CommandByte;

impl Command {
	pub fn from_stream(stream: &mut TcpStream) -> Result<Self, StreamError> {
		let mut reader = StreamReader::new(stream);

		match reader.read_u8()? {
			CommandByte::PING => Ok(Command::Ping),
			CommandByte::VERSION => Ok(Command::Version),

			CommandByte::GET => {
				let key = reader.read_buf()?;
				Ok(Command::Get(hash(&key)))
			},

			CommandByte::SET => {
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

			CommandByte::DEL => {
				let key = reader.read_buf()?;
				Ok(Command::Del(hash(&key)))
			},

			CommandByte::PEEK => {
				let key = reader.read_buf()?;
				Ok(Command::Peek(hash(&key)))
			},

			CommandByte::WIPE => Ok(Command::Wipe),

			CommandByte::RESIZE => {
				let size = reader.read_u64()?;
				Ok(Command::Resize(size))
			},

			CommandByte::POLICY => {
				let byte = reader.read_u8()?;

				let policy = match byte {
					0 => CachePolicy::Lfu,
					1 => CachePolicy::Fifo,
					2 => CachePolicy::Lru,
					3 => CachePolicy::Mru,

					_ => {
						return Err(StreamError::new(
							ErrorKind::InvalidData,
							"Invalid policy."
						))
					},
				};

				Ok(Command::Policy(policy))
			},

			CommandByte::STATS => Ok(Command::Stats),

			_ => Err(StreamError::new(
				ErrorKind::InvalidData,
				"Invalid command."
			))
		}
	}
}

impl CommandByte {
	const PING: u8 = 0;
	const VERSION: u8 = 1;

	const GET: u8 = 2;
	const SET: u8 = 3;
	const DEL: u8 = 4;
	const PEEK: u8 = 5;

	const WIPE: u8 = 6;

	const RESIZE: u8 = 7;
	const POLICY: u8 = 8;

	const STATS: u8 = 9;
}

fn hash(data: &Buffer) -> u32 {
	murmur3::hash32(data)
}
