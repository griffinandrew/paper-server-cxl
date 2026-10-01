/*
 * Copyright (c) Kia Shakiba
 *
 * This source code is licensed under the GNU AGPLv3 license found in the
 * LICENSE file in the root directory of this source tree.
 */

use std::{
	hash::{DefaultHasher, Hash, Hasher},
	io::{self, Read, Write},
	net::{Shutdown, TcpStream},
	time::{Duration, Instant},
};

use paper_utils::stream::StreamError;

use crate::{command::Command, error::ServerError};

pub struct Connection {
	stream: TcpStream,

	auth_token:    Option<u64>,
	is_authorized: bool,
}

impl Connection {
	pub fn new(stream: TcpStream, auth_token: Option<u64>) -> Self {
		let is_authorized = auth_token.is_none();

		Connection {
			stream,

			auth_token,
			is_authorized,
		}
	}

	pub fn close(&self) -> Result<(), ServerError> {
		self.stream
			.shutdown(Shutdown::Both)
			.map_err(|_| ServerError::Internal)
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

		self.is_authorized = self
			.auth_token
			.is_some_and(|token| token == s.finish());

		self.is_authorized
	}

	pub fn get_command(&mut self) -> Result<Command, ServerError> {
		Command::from_stream(&mut self.stream).map_err(|err| match err {
			StreamError::InvalidStream | StreamError::ClosedStream => ServerError::Disconnected,

			_ => ServerError::InvalidCommand(err.to_string()),
		})
	}

	pub fn send_response(&mut self, buf: &[u8]) -> Result<(), ServerError> {
		self.stream
			.write_all(buf)
			.map_err(|_| ServerError::InvalidResponse)
	}

	/// The socket itself, for a value to be read straight off it.
	pub fn stream_mut(&mut self) -> &mut TcpStream {
		&mut self.stream
	}

	/// Arms the socket's receive timeout (SO_RCVTIMEO), the longest one read
	/// waits for data, or with `None` disarms it. A connection is idle between
	/// commands and waits for the next one without limit, so it is armed only
	/// while a SET's value and TTL are being read.
	pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
		self.stream.set_read_timeout(timeout)
	}

	/// A little-endian u32 straight off the socket: a SET's TTL.
	pub fn read_u32(&mut self) -> io::Result<u32> {
		let mut bytes = [0u8; 4];

		self.stream.read_exact(&mut bytes)?;

		Ok(u32::from_le_bytes(bytes))
	}

	/// Reads and discards `len` bytes, through a scratch buffer of a few KiB on
	/// the stack, so that a request the server will not take is consumed to its
	/// end and the connection stays in step. Gives up at `deadline`, between
	/// reads; each read also waits at most the receive timeout, if one is armed.
	pub fn skip(&mut self, len: u64, deadline: Instant) -> io::Result<()> {
		let mut scratch = [0u8; 16 * 1024];
		let mut left = len;

		while left > 0 {
			if Instant::now() >= deadline {
				return Err(io::ErrorKind::TimedOut.into());
			}

			let take = left.min(scratch.len() as u64) as usize;

			match self.stream.read(&mut scratch[..take]) {
				Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
				Ok(read) => left -= read as u64,
				Err(err) if err.kind() == io::ErrorKind::Interrupted => {},
				Err(err) => return Err(err),
			}
		}

		Ok(())
	}
}
