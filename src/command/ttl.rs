use crate::{
	error::ServerError,
	server::CacheRef,
	connection::Connection,
	frame::Frame,
	parse::Parse,
	command::{Command, hash},
};

pub struct Ttl {
	key: u64,
	ttl: Option<u32>,
}

impl Command for Ttl {
	fn parse_frames(parse: &mut Parse) -> Result<Self, ServerError> {
		let key = parse.next_bytes()?;
		let hashed = hash(&key);

		let ttl = match parse.next_u32()? {
			0 => None,
			ttl => Some(ttl),
		};

		let command = Ttl {
			key: hashed,
			ttl,
		};

		Ok(command)
	}

	async fn apply(self, dst: &mut Connection, cache: &CacheRef) -> Result<(), ServerError> {
		cache.ttl(self.key, self.ttl)?;

		let frame = Frame::Array(vec![Frame::Bool(true)]);
		dst.write_frame(&frame).await?;

		Ok(())
	}
}
