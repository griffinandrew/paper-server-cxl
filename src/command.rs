/*
 * Copyright (c) Kia Shakiba
 *
 * This source code is licensed under the GNU AGPLv3 license found in the
 * LICENSE file in the root directory of this source tree.
 */

use std::time::Instant;
use std::time::Duration;
use std::hint::black_box;

use std::net::TcpStream;

use paper_utils::{
	stream::{Buffer, StreamReader, StreamError},
	command::CommandByte,
};

pub enum Command {
	Ping,
	Version,

	Auth(Buffer),

	Get(Buffer),
	Set(Buffer, Buffer, Option<u32>),
	Del(Buffer),

	Has(Buffer),
	Peek(Buffer),
	Ttl(Buffer, Option<u32>),
	Size(Buffer),

	Wipe,

	Resize(u64),
	Policy(String),

	Status,
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
				// Simulate latency for the SET command
				// latency is both the time it takes to read the key and value, not just the vakue 
				// this simualtes all objects in CXL tier.... 
				let start = std::time::Instant::now();
				let key = reader.read_buf()?;
				let value = reader.read_buf()?;
				let elapsed = start.elapsed().as_nanos() as u64;

				// Spin for the duration of recorded access
				let end = Instant::now() + Duration::from_nanos(elapsed);
				while black_box(Instant::now()) < black_box(end) {
					//println!("CxlPtr deref spin loop");
					black_box(std::hint::spin_loop());
				}
				//ensure that the spin loop is actually triggering and running for double the time
				let total_duration = start.elapsed().as_nanos() as u64;
				let expected_time = elapsed * 2;
				black_box(assert!(total_duration >= expected_time, "CxlPtr deref took less time than expected IN SERVER: {} < {}", total_duration, expected_time));


				let ttl = match reader.read_u32()? {
					0 => None,
					value => Some(value),
				};

				Ok(Command::Set(key, value, ttl))
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

			_ => Err(StreamError::InvalidData),
		}
	}
}
