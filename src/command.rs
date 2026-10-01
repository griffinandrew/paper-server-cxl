/*
 * Copyright (c) Kia Shakiba
 *
 * This source code is licensed under the GNU AGPLv3 license found in the
 * LICENSE file in the root directory of this source tree.
 */

use std::net::TcpStream;

use paper_utils::{
	command::CommandByte,
	stream::{Buffer, StreamError, StreamReader, read_stack_buf},
};

use crate::keybuf::KeyBuf;

/// The command byte the server answers its OWN statistics on.
///
/// Deliberately outside the protocol's 0..=13 range: a stock client never sends
/// it, and the answer is a plain text buffer rather than the fixed STATUS
/// frame, which has no room for tier fields.
pub const COMMAND_SELF_STATS: u8 = 200;

pub enum Command {
	Ping,
	Version,

	Auth(Buffer),

	// The commands that name a key do not carry it: it is in the `KeyBuf`
	// `from_stream` was given, which the connection keeps and reuses, so a
	// request allocates nothing for it (see `keybuf`).
	Get,

	/// The length of the value. The value itself and the TTL that follows it
	/// are still on the socket: the SET arm reads them, so that the cache can
	/// decide whether to take the set before the bytes are read (see `set`).
	Set(u32),
	Del,

	Has,
	Peek,
	Ttl(Option<u32>),
	Size,

	Wipe,

	Resize(u64),
	Policy(String),

	Status,

	SelfStats,
}

impl Command {
	/// Reads a command off the socket. The key of one that names a key is read
	/// into `key`, replacing what it held.
	pub fn from_stream(stream: &mut TcpStream, key: &mut KeyBuf) -> Result<Self, StreamError> {
		match read_u8(stream)? {
			CommandByte::PING => Ok(Command::Ping),
			CommandByte::VERSION => Ok(Command::Version),

			CommandByte::AUTH => {
				let token = StreamReader::new(stream).read_buf()?;
				Ok(Command::Auth(token))
			},

			CommandByte::GET => {
				read_key(stream, key)?;
				Ok(Command::Get)
			},

			CommandByte::SET => {
				read_key(stream, key)?;
				let len = read_u32(stream)?;

				Ok(Command::Set(len))
			},

			CommandByte::DEL => {
				read_key(stream, key)?;
				Ok(Command::Del)
			},

			CommandByte::HAS => {
				read_key(stream, key)?;
				Ok(Command::Has)
			},

			CommandByte::PEEK => {
				read_key(stream, key)?;
				Ok(Command::Peek)
			},

			CommandByte::TTL => {
				read_key(stream, key)?;

				let ttl = match read_u32(stream)? {
					0 => None,
					value => Some(value),
				};

				Ok(Command::Ttl(ttl))
			},

			CommandByte::SIZE => {
				read_key(stream, key)?;
				Ok(Command::Size)
			},

			CommandByte::WIPE => Ok(Command::Wipe),

			CommandByte::RESIZE => {
				let size = StreamReader::new(stream).read_u64()?;
				Ok(Command::Resize(size))
			},

			CommandByte::POLICY => {
				let policy_str = StreamReader::new(stream).read_string()?;
				Ok(Command::Policy(policy_str))
			},

			CommandByte::STATUS => Ok(Command::Status),

			COMMAND_SELF_STATS => Ok(Command::SelfStats),

			_ => Err(StreamError::InvalidData),
		}
	}
}

/// A byte off the socket, as `StreamReader::read_u8` reads it.
fn read_u8(stream: &mut TcpStream) -> Result<u8, StreamError> {
	Ok(read_stack_buf::<1>(stream)?[0])
}

/// A little-endian u32 off the socket, as `StreamReader::read_u32` reads it.
fn read_u32(stream: &mut TcpStream) -> Result<u32, StreamError> {
	Ok(u32::from_le_bytes(read_stack_buf::<4>(stream)?))
}

/// A key off the socket, `[len: u32][bytes]`, into the connection's key buffer
/// -- what `StreamReader::read_buf` reads, into a `Box` of its own that is
/// allocated and zeroed for every request. Any failure to read is a closed
/// stream, as it is there.
fn read_key(stream: &mut TcpStream, key: &mut KeyBuf) -> Result<(), StreamError> {
	let len = read_u32(stream)? as usize;

	key.read_from(stream, len).map_err(|_| StreamError::ClosedStream)
}
