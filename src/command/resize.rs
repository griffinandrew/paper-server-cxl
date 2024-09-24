use crate::{
	error::ServerError,
	server::CacheRef,
	connection::Connection,
	frame::Frame,
	parse::Parse,
	command::{Command, hash},
};

pub struct Resize {
	size: u64,
}

impl Command for Resize {
	fn parse_frames(parse: &mut Parse) -> Result<Self, ServerError> {
		let size = parse.next_u64()?;

		let command = Resize {
			size,
		};

		Ok(command)
	}

	async fn apply(self, dst: &mut Connection, cache: &CacheRef) -> Result<(), ServerError> {
		cache.resize(self.size)?;

		let frame = Frame::Array(vec![Frame::Bool(true)]);
		dst.write_frame(&frame).await?;

		Ok(())
	}
}
