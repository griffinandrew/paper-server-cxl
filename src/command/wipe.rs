use crate::{
	error::ServerError,
	server::CacheRef,
	connection::Connection,
	frame::Frame,
	parse::Parse,
	command::Command,
};

pub struct Wipe;

impl Command for Wipe {
	fn parse_frames(_: &mut Parse) -> Result<Self, ServerError> {
		Ok(Wipe)
	}

	async fn apply(self, dst: &mut Connection, cache: &CacheRef) -> Result<(), ServerError> {
		cache.wipe()?;

		let frame = Frame::Array(vec![Frame::Bool(true)]);
		dst.write_frame(&frame).await?;

		Ok(())
	}
}
