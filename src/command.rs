use std::io;
use tokio::net::TcpStream;
use crate::server_error::{ServerError, ErrorKind};

pub enum Command {
	Ping,

	Get(u64),
	Set(u64, String),
	Del(u64),

	Resize(u64),
	Policy(u8),
}

impl Command {
	pub fn from_stream(stream: &TcpStream, ip: &str) -> Result<Self, ServerError> {
		let (command_byte, size) = read_headers(stream, ip)?;

		match command_byte {
			0 => Ok(Command::Ping),

			1 => Ok(Command::Get(123)),
			2 => Ok(Command::Set(123, "456".to_owned())),
			3 => Ok(Command::Del(123)),

			4 => Ok(Command::Resize(0)),
			5 => Ok(Command::Policy(0)),

			_ => Err(ServerError::new(
				ErrorKind::InvalidCommand,
				"Invalid command."
			))
		}
	}
}

fn read_headers(stream: &TcpStream, ip: &str) -> Result<(u8, u32), ServerError> {
	let mut buf = [0u8; 5];

	loop {
		match stream.try_read(&mut buf) {
			Ok(0) => {
				return Err(ServerError::new(
					ErrorKind::ConnectionLost,
					&format!("\x1B[31mDisconnected\x1B[0m:\t<{}>", ip)
				));
			},

			Ok(size) => {
				if size != 5 {
					return Err(ServerError::new(
						ErrorKind::EmptyBuf,
						"Could not read response."
					));
				}

				let size = u32::from_le_bytes([
					buf[1],
					buf[2],
					buf[3],
					buf[4],
				]);

				return Ok((buf[0], size));
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

fn read_data(stream: &TcpStream, buf_size: usize) -> Result<String, ServerError> {
	let mut data = Vec::<u8>::with_capacity(buf_size);
	let mut buf = [0u8; 4096];
	let mut read_bytes: usize = 0;

	loop {
		match stream.try_read(&mut buf) {
			Ok(0) => {
				if read_bytes == buf_size {
					return Ok(String::from_utf8(data).unwrap());
				}

				return Err(ServerError::new(
					ErrorKind::EmptyBuf,
					"Could not read response."
				));
			},

			Ok(size) => {
				data.extend_from_slice(&buf[0..size]);
				read_bytes += size;

				if size == buf_size {
					return Ok(String::from_utf8(data).unwrap());
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
