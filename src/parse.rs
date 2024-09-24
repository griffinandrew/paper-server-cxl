use std::vec;
use bytes::Bytes;

use crate::{
	frame::Frame,
	error::{ServerError, ParseError},
};

#[derive(Debug)]
pub struct Parse {
	parts: vec::IntoIter<Frame>,
}

impl Parse {
	pub fn new(frame: Frame) -> Result<Self, ParseError> {
		let frames = match frame {
			Frame::Array(frames) => frames,
			_ => return Err(ParseError::Server(ServerError::Internal)),
		};

		let parse = Parse {
			parts: frames.into_iter(),
		};

		Ok(parse)
	}

	fn next_frame(&mut self) -> Result<Frame, ParseError> {
		self.parts.next().ok_or(ParseError::EndOfStream)
	}

	pub fn next_byte(&mut self) -> Result<u8, ParseError> {
		match self.next_frame()? {
			Frame::Byte(byte) => Ok(byte),
			_ => Err(ParseError::InvalidProtocol),
		}
	}

	pub fn next_u32(&mut self) -> Result<u32, ParseError> {
		match self.next_frame()? {
			Frame::U32(value) => Ok(value),
			_ => Err(ParseError::InvalidProtocol),
		}
	}

	pub fn next_u64(&mut self) -> Result<u64, ParseError> {
		match self.next_frame()? {
			Frame::U64(value) => Ok(value),
			_ => Err(ParseError::InvalidProtocol),
		}
	}

	pub fn next_bytes(&mut self) -> Result<Bytes, ParseError> {
		match self.next_frame()? {
			Frame::Bytes(bytes) => Ok(bytes),
			_ => Err(ParseError::InvalidProtocol),
		}
	}

	pub fn finish(mut self) -> Result<(), ParseError> {
		match self.next_frame() {
			Ok(_) => Err(ParseError::InvalidProtocol),
			Err(_) => Ok(()),
		}
	}
}
