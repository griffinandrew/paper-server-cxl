use std::io::{self, Cursor};
use bytes::{Buf, BytesMut};

use tokio::{
	io::{BufWriter, AsyncReadExt, AsyncWriteExt},
	net::TcpStream,
};

use crate::{
	error::{ServerError, FrameError},
	frame::Frame,
};

#[derive(Debug)]
pub struct Connection {
	stream: BufWriter<TcpStream>,
	buffer: BytesMut,
}

impl Connection {
	pub fn new(socket: TcpStream) -> Result<Self, ServerError> {
		socket.set_nodelay(true)?;

		let connection = Connection {
			stream: BufWriter::new(socket),
			buffer: BytesMut::with_capacity(4096),
		};

		Ok(connection)
	}

	pub async fn read_frame(&mut self) -> Result<Option<Frame>, FrameError> {
		loop {
			if let Some(frame) = self.parse_frame()? {
				return Ok(Some(frame));
			}

			let num_read_bytes = self.stream
				.read_buf(&mut self.buffer).await
				.map_err(|_| FrameError::Server(ServerError::Internal))?;

			if num_read_bytes == 0 {
				if self.buffer.is_empty() {
					return Ok(None);
				} else {
					return Err(FrameError::Server(ServerError::InvalidConnection));
				}
			}
		}
	}

	fn parse_frame(&mut self) -> Result<Option<Frame>, FrameError> {
		let mut buf = Cursor::new(&self.buffer[..]);

		match Frame::check(&mut buf) {
			Ok(_) => {
				let len = buf.position() as usize;
				buf.set_position(0);

				let frame = Frame::parse(&mut buf)?;
				self.buffer.advance(len);

				Ok(Some(frame))
			},

			Err(FrameError::Incomplete) => Ok(None),
			Err(err) => Err(err),
		}
	}

	pub async fn write_frame(&mut self, frame: &Frame) -> io::Result<()> {
		match frame {
			Frame::Array(frames) => {
				for frame in frames {
					self.write_value(frame).await?;
				}
			},

			_ => self.write_value(frame).await?,
		}

		self.stream.flush().await
	}

	async fn write_value(&mut self, frame: &Frame) -> io::Result<()> {
		match frame {
			Frame::Bool(value) => {
				if *value {
					self.stream.write_u8(b"!"[0]).await?;
				} else {
					self.stream.write_u8(b"?"[0]).await?;
				}
			},

			Frame::Byte(value) => {
				self.stream.write_u8(*value).await?;
			},

			Frame::U32(value) => {
				self.stream.write_u32_le(*value).await?;
			},

			Frame::U64(value) => {
				self.stream.write_u64_le(*value).await?;
			},

			Frame::F64(value) => {
				self.stream.write_f64_le(*value).await?;
			},

			Frame::Bytes(value) => {
				self.stream.write_u32_le(value.len() as u32).await?;
				self.stream.write_all(value).await?;
			},

			Frame::Array(_) => unreachable!(),
		}

		Ok(())
	}
}
