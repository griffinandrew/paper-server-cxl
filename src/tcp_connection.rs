use std::{
	io::Write,
	hash::{DefaultHasher, Hash, Hasher},
	net::TcpStream,
};

use paper_utils::stream::StreamError;

use crate::{
	error::ServerError,
	command::Command,
};

pub struct TcpConnection {
	stream: TcpStream,

	auth_token: Option<u64>,
	is_authorized: bool,
}

impl TcpConnection {
	pub fn new(
		stream: TcpStream,
		auth_token: Option<u64>,
	) -> Self {
		let is_authorized = auth_token.is_none();

		TcpConnection {
			stream,

			auth_token,
			is_authorized,
		}
	}

	pub fn is_authorized(&self) -> bool {
		self.is_authorized
	}

	pub fn authorize(&mut self, value: &str) -> bool {
		if self.is_authorized {
			return true;
		}

		let mut s = DefaultHasher::new();
		value.hash(&mut s);

		self.is_authorized = self.auth_token
			.is_some_and(|token| token == s.finish());

		self.is_authorized
	}

	pub fn get_command(&mut self) -> Result<Command, ServerError> {
		Command::from_stream(&mut self.stream).map_err(|err| match err {
			StreamError::InvalidStream | StreamError::ClosedStream
				=> ServerError::Disconnected,

			_ => ServerError::InvalidCommand(err.to_string()),
		})
	}

	pub fn send_response(&mut self, buf: &[u8]) -> Result<(), ServerError> {
		self.stream
			.write_all(buf)
			.map_err(|_| ServerError::InvalidResponse)
	}
}
