use bytes::Bytes;

use crate::{
	error::ServerError,
	server::CacheRef,
	connection::Connection,
	frame::Frame,
	parse::Parse,
	command::Command,
};

pub struct Auth {
	token: Bytes,
}

impl Command for Auth {
	fn parse_frames(_: &mut Parse) -> Result<Self, ServerError> {
		todo!();
	}

	async fn apply(self, dst: &mut Connection, cache: &CacheRef) -> Result<(), ServerError> {
		/*let mut frames = vec![Frame::Bool(true)];
		frames.push(Frame::Bytes(Bytes::from(cache.version())));

		let frame = Frame::Array(frames);
		dst.write_frame(&frame).await?;*/

		Ok(())
	}
}
