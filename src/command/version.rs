use bytes::Bytes;

use crate::{
	error::ServerError,
	server::CacheRef,
	connection::Connection,
	frame::Frame,
	parse::Parse,
	command::Command,
};

pub struct Version;

impl Command for Version {
	fn parse_frames(_: &mut Parse) -> Result<Self, ServerError> {
		Ok(Version)
	}

	async fn apply(self, dst: &mut Connection, cache: &CacheRef) -> Result<(), ServerError> {
		let version = cache.version();

		let frames = vec![
			Frame::Bool(true),
			Frame::Bytes(Bytes::from(version)),
		];

		let frame = Frame::Array(frames);
		dst.write_frame(&frame).await?;

		Ok(())
	}
}
