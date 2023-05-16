use std::io;
use std::io::Cursor;
use tokio::net::TcpStream;
use byteorder::{LittleEndian, ReadBytesExt};
use fasthash::murmur3;
use crate::server_error::{ServerError, ErrorKind};

pub enum Command {
	Ping,

	Get(u32),
	Set(u32, String),
	Del(u32),

	Resize(u64),
	Policy(u8),
}

impl Command {
	pub fn from_stream(stream: &TcpStream) -> Result<Self, ServerError> {
		let command_byte = read_command(stream)?;

		match command_byte {
			0 => Ok(Command::Ping),

			1 => {
				let key = read_key(stream)?;

				Ok(Command::Get(key))
			},

			2 => {
				let key = read_key(stream)?;
				let value = read_value(stream)?;

				Ok(Command::Set(key, value))
			},

			3 => {
				let key = read_key(stream)?;

				Ok(Command::Del(key))
			}

			4 => Ok(Command::Resize(0)),
			5 => Ok(Command::Policy(0)),

			_ => Err(ServerError::new(
				ErrorKind::InvalidCommand,
				"Invalid command."
			))
		}
	}
}

fn read_command(stream: &TcpStream) -> Result<u8, ServerError> {
	let buf = read_buf(stream, 1)?;
	Ok(buf[0])
}

fn read_key(stream: &TcpStream) -> Result<u32, ServerError> {
	let size_buf = read_buf(stream, 4)?;
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

	let key_buf = read_buf(stream, size as usize)?;

	Ok(hash(&key_buf))
}

fn read_value(stream: &TcpStream) -> Result<String, ServerError> {
	let size_buf = read_buf(stream, 4)?;
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

	let value_buf = read_buf(stream, size as usize)?;

	Ok(String::from_utf8(value_buf).unwrap())
}

fn read_buf(stream: &TcpStream, buf_size: usize) -> Result<Vec<u8>, ServerError> {
	let mut buf = vec![0u8; buf_size];
	let mut read_bytes: usize = 0;

	loop {
		match stream.try_read(&mut buf) {
			Ok(0) => {
				if read_bytes == buf_size {
					return Ok(buf);
				}

				return Err(ServerError::new(
					ErrorKind::EmptyBuf,
					"Could not read response."
				));
			},

			Ok(size) => {
				read_bytes += size;

				if size == buf_size {
					return Ok(buf);
				}

				continue;
			},

			Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
				continue;
			},

			Err(_) => {
				return Err(ServerError::new(
					ErrorKind::InvalidStream,
					"Could not read response."
				));
			},
		}
	}
}

fn hash(data: &Vec<u8>) -> u32 {
	murmur3::hash32(data)
}
