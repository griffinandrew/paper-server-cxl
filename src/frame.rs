use std::io::Cursor;
use bytes::{Buf, Bytes};
use paper_utils::command::CommandByte;
use crate::error::FrameError;

#[derive(Debug)]
pub enum Frame {
	Bool(bool),
	Byte(u8),
	U32(u32),
	U64(u64),
	F64(f64),
	Bytes(Bytes),

	Array(Vec<Frame>),
}

impl Frame {
	pub fn check(src: &mut Cursor<&[u8]>) -> Result<(), FrameError> {
		let command_byte = get_u8(src)?;

		match command_byte {
			CommandByte::PING
				| CommandByte::VERSION
				| CommandByte::WIPE
				| CommandByte::STATS => Ok(()),

			CommandByte::AUTH => {
				skip_bytes(src)?;
				Ok(())
			},

			CommandByte::GET
				| CommandByte::DEL
				| CommandByte::HAS
				| CommandByte::PEEK
				| CommandByte::SIZE => {

				skip_bytes(src)?;
				Ok(())
			},

			CommandByte::SET => {
				skip_bytes(src)?;
				skip_bytes(src)?;
				get_u32(src)?;

				Ok(())
			},

			CommandByte::TTL => {
				skip_bytes(src)?;
				get_u32(src)?;

				Ok(())
			},

			CommandByte::RESIZE => {
				get_u64(src)?;
				Ok(())
			},

			CommandByte::POLICY => {
				get_u8(src)?;
				Ok(())
			},

			_ => unimplemented!(),
		}

	}

	pub fn parse(src: &mut Cursor<&[u8]>) -> Result<Frame, FrameError> {
		let mut frames = Vec::<Frame>::new();
		let command_byte = get_u8(src)?;

		frames.push(Frame::Byte(command_byte));

		match command_byte {
			CommandByte::PING
				| CommandByte::VERSION
				| CommandByte::WIPE
				| CommandByte::STATS => {},

			CommandByte::AUTH
				| CommandByte::GET
				| CommandByte::DEL
				| CommandByte::HAS
				| CommandByte::PEEK
				| CommandByte::SIZE => {

				let bytes = get_bytes(src)?;
				frames.push(Frame::Bytes(bytes));
			},

			CommandByte::SET => {
				let key = Frame::Bytes(get_bytes(src)?);
				let value = Frame::Bytes(get_bytes(src)?);
				let ttl = Frame::U32(get_u32(src)?);

				frames.push(key);
				frames.push(value);
				frames.push(ttl);
			},

			CommandByte::TTL => {
				let key = Frame::Bytes(get_bytes(src)?);
				let ttl = Frame::U32(get_u32(src)?);

				frames.push(key);
				frames.push(ttl);
			},

			CommandByte::RESIZE => {
				let size = Frame::U64(get_u64(src)?);
				frames.push(size);
			},

			CommandByte::POLICY => {
				let policy = Frame::Byte(get_u8(src)?);
				frames.push(policy);
			},

			_ => unimplemented!(),
		}

		Ok(Frame::Array(frames))
	}
}

fn get_u8(src: &mut Cursor<&[u8]>) -> Result<u8, FrameError> {
	if !src.has_remaining() {
		return Err(FrameError::Incomplete);
	}

	Ok(src.get_u8())
}

fn get_u32(src: &mut Cursor<&[u8]>) -> Result<u32, FrameError> {
	if src.remaining() < 4 {
		return Err(FrameError::Incomplete);
	}

	Ok(src.get_u32_le())
}

fn get_u64(src: &mut Cursor<&[u8]>) -> Result<u64, FrameError> {
	if src.remaining() < 8 {
		return Err(FrameError::Incomplete);
	}

	Ok(src.get_u64_le())
}

fn get_bytes(src: &mut Cursor<&[u8]>) -> Result<Bytes, FrameError> {
	let size = get_u32(src)? as usize;

	if src.remaining() < size {
		return Err(FrameError::Incomplete);
	}

	let bytes = Bytes::copy_from_slice(&src.chunk()[..size]);
	src.advance(size);

	Ok(bytes)
}

fn skip_bytes(src: &mut Cursor<&[u8]>) -> Result<(), FrameError> {
	let size = get_u32(src)? as usize;

	if src.remaining() < size {
		return Err(FrameError::Incomplete);
	}

	src.advance(size);

	Ok(())
}
