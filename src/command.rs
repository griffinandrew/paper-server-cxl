use std::net::TcpStream;
use fasthash::murmur3;
use paper_cache::policy::Policy as CachePolicy;

use paper_utils::{
	stream::{Buffer, StreamReader, StreamError, ErrorKind},
	command::CommandByte,
	policy::PolicyByte,
};

use crate::server_object::ServerObject;

pub enum Command {
	Ping,
	Version,

	Get(u32),
	Set(u32, ServerObject, Option<u32>),
	Del(u32),

	Has(u32),
	Peek(u32),

	Wipe,

	Resize(u64),
	Policy(CachePolicy),

	Stats,
}

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

			CommandByte::HAS => {
				let key = reader.read_buf()?;
				Ok(Command::Has(hash(&key)))
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
					PolicyByte::LFU => CachePolicy::Lfu,
					PolicyByte::FIFO => CachePolicy::Fifo,
					PolicyByte::LRU => CachePolicy::Lru,
					PolicyByte::MRU => CachePolicy::Mru,

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

fn hash(data: &Buffer) -> u32 {
	murmur3::hash32(data)
}
