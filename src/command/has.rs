use crate::{
	error::ServerError,
	server::CacheRef,
	connection::Connection,
	frame::Frame,
	parse::Parse,
	command::{Command, hash},
};

pub struct Has {
	key: u64,
}

impl Command for Has {
	fn parse_frames(parse: &mut Parse) -> Result<Self, ServerError> {
		let key = parse.next_bytes()?;
		let hashed = hash(&key);

		let command = Has {
			key: hashed,
		};

		Ok(command)
	}

	async fn apply(self, dst: &mut Connection, cache: &CacheRef) -> Result<(), ServerError> {
		let has = cache.has(self.key);

		let frames = vec![
			Frame::Bool(true),
			Frame::Bool(has),
		];

		let frame = Frame::Array(frames);
		dst.write_frame(&frame).await?;

		Ok(())
	}
}
