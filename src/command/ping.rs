use bytes::Bytes;

use crate::{
	error::ServerError,
	server::CacheRef,
	connection::Connection,
	frame::Frame,
	parse::Parse,
	command::Command,
};

pub struct Ping;

impl Command for Ping {
	fn parse_frames(_: &mut Parse) -> Result<Self, ServerError> {
		Ok(Ping)
	}

	async fn apply(self, dst: &mut Connection, _: &CacheRef) -> Result<(), ServerError> {
		let frames = vec![
			Frame::Bool(true),
			Frame::Bytes(Bytes::copy_from_slice(b"pong")),
		];

		let frame = Frame::Array(frames);
		dst.write_frame(&frame).await?;

		Ok(())
	}
}
