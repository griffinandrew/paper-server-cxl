/*
 * Copyright (c) Kia Shakiba
 *
 * This source code is licensed under the GNU AGPLv3 license found in the
 * LICENSE file in the root directory of this source tree.
 */

use std::net::TcpStream;

use paper_utils::{
	command::CommandByte,
	stream::{Buffer, StreamError, StreamReader},
};

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

	Get(Buffer),

	/// The key and the length of the value. The value itself and the TTL that
	/// follows it are still on the socket: the SET arm reads them, so that the
	/// cache can decide whether to take the set before the bytes are read (see
	/// `set`).
	Set(Buffer, u32),
	Del(Buffer),

	Has(Buffer),
	Peek(Buffer),
	Ttl(Buffer, Option<u32>),
	Size(Buffer),

	Wipe,

	Resize(u64),
	Policy(String),

	Status,

	SelfStats,
}

impl Command {
	pub fn from_stream(stream: &mut TcpStream) -> Result<Self, StreamError> {
		let mut reader = StreamReader::new(stream);

		match reader.read_u8()? {
			CommandByte::PING => Ok(Command::Ping),
			CommandByte::VERSION => Ok(Command::Version),

			CommandByte::AUTH => {
				let token = reader.read_buf()?;
				Ok(Command::Auth(token))
			},

			CommandByte::GET => {
				let key = reader.read_buf()?;
				Ok(Command::Get(key))
			},

			CommandByte::SET => {
				let key = reader.read_buf()?;
				let len = reader.read_u32()?;

				Ok(Command::Set(key, len))
			},

			CommandByte::DEL => {
				let key = reader.read_buf()?;
				Ok(Command::Del(key))
			},

			CommandByte::HAS => {
				let key = reader.read_buf()?;
				Ok(Command::Has(key))
			},

			CommandByte::PEEK => {
				let key = reader.read_buf()?;
				Ok(Command::Peek(key))
			},

			CommandByte::TTL => {
				let key = reader.read_buf()?;

				let ttl = match reader.read_u32()? {
					0 => None,
					value => Some(value),
				};

				Ok(Command::Ttl(key, ttl))
			},

			CommandByte::SIZE => {
				let key = reader.read_buf()?;
				Ok(Command::Size(key))
			},

			CommandByte::WIPE => Ok(Command::Wipe),

			CommandByte::RESIZE => {
				let size = reader.read_u64()?;
				Ok(Command::Resize(size))
			},

			CommandByte::POLICY => {
				let policy_str = reader.read_string()?;
				Ok(Command::Policy(policy_str))
			},

			CommandByte::STATUS => Ok(Command::Status),

			COMMAND_SELF_STATS => Ok(Command::SelfStats),

			_ => Err(StreamError::InvalidData),
		}
	}
}
