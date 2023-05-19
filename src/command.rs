use std::io::Cursor;
use tokio::net::TcpStream;
use byteorder::{LittleEndian, ReadBytesExt};
use fasthash::murmur3;
use paper_core::stream::{Buffer, read_buf as stream_read_buf};
use paper_core::stream_error::{ErrorKind as StreamErrorKind};
use paper_cache::policy::Policy as CachePolicy;
use crate::server_error::{ServerError, ErrorKind};

pub enum Command {
	Ping,

	Get(u32),
	Set(u32, String, Option<u32>),
	Del(u32),

	Resize(u64),
	Policy(&'static CachePolicy),

	Stats,
}

impl Command {
	pub async fn from_stream(stream: &TcpStream) -> Result<Self, ServerError> {
		let command_byte = read_u8(stream).await?;

		match command_byte {
			0 => Ok(Command::Ping),

			1 => {
				let key = read_key(stream).await?;

				Ok(Command::Get(key))
			},

			2 => {
				let key = read_key(stream).await?;
				let value = read_value(stream).await?;

				let ttl = match read_u32(stream).await? {
					0 => None,
					value => Some(value),
				};

				Ok(Command::Set(key, value, ttl))
			},

			3 => {
				let key = read_key(stream).await?;

				Ok(Command::Del(key))
			}

			4 => {
				let size = read_u64(stream).await?;

				Ok(Command::Resize(size))
			},

			5 => {
				let byte = read_u8(stream).await?;

				let policy = match byte {
					0 => &CachePolicy::Lru,
					1 => &CachePolicy::Mru,

					_ => {
						return Err(ServerError::new(
							ErrorKind::InvalidCommand,
							"Invalid command."
						))
					},
				};

				Ok(Command::Policy(policy))
			},

			6 => Ok(Command::Stats),

			_ => Err(ServerError::new(
				ErrorKind::InvalidCommand,
				"Invalid command."
			))
		}
	}
}

async fn read_u8(stream: &TcpStream) -> Result<u8, ServerError> {
	let buf = read_buf(stream, 1).await?;
	Ok(buf[0])
}

async fn read_key(stream: &TcpStream) -> Result<u32, ServerError> {
	let size_buf = read_buf(stream, 4).await?;
	let mut rdr = Cursor::new(size_buf);

	let size = match rdr.read_u32::<LittleEndian>() {
		Ok(size) => size,

		Err(_) => {
			return Err(ServerError::new(
				ErrorKind::InvalidStream,
				"Invalid data in stream."
			));
		}
	};

	let key_buf = read_buf(stream, size as usize).await?;

	Ok(hash(&key_buf))
}

async fn read_value(stream: &TcpStream) -> Result<String, ServerError> {
	let size_buf = read_buf(stream, 4).await?;
	let mut rdr = Cursor::new(size_buf);

	let size = match rdr.read_u32::<LittleEndian>() {
		Ok(size) => size,

		Err(_) => {
			return Err(ServerError::new(
				ErrorKind::InvalidStream,
				"Invalid data in stream."
			));
		}
	};

	let value_buf = read_buf(stream, size as usize).await?;

	Ok(String::from_utf8(value_buf).unwrap())
}

async fn read_u32(stream: &TcpStream) -> Result<u32, ServerError> {
	let buf = read_buf(stream, 4).await?;
	let mut rdr = Cursor::new(buf);

	match rdr.read_u32::<LittleEndian>() {
		Ok(value) => Ok(value),

		Err(_) => {
			return Err(ServerError::new(
				ErrorKind::InvalidStream,
				"Invalid data in stream."
			));
		}
	}
}

async fn read_u64(stream: &TcpStream) -> Result<u64, ServerError> {
	let buf = read_buf(stream, 8).await?;
	let mut rdr = Cursor::new(buf);

	match rdr.read_u64::<LittleEndian>() {
		Ok(value) => Ok(value),

		Err(_) => {
			return Err(ServerError::new(
				ErrorKind::InvalidStream,
				"Invalid data in stream."
			));
		}
	}
}

async fn read_buf(stream: &TcpStream, size: usize) -> Result<Buffer, ServerError> {
	match stream_read_buf(stream, size).await {
		Ok(buf) => Ok(buf),

		Err(ref err) if err.kind() == &StreamErrorKind::Disconnected => Err(ServerError::new(
			ErrorKind::Disconnected,
			"Disconnected from client."
		)),

		Err(_) => Err(ServerError::new(
			ErrorKind::InvalidStream,
			"Invalid data in stream."
		))
	}
}

fn hash(data: &Buffer) -> u32 {
	murmur3::hash32(data)
}
