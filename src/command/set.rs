use crate::{
	error::ServerError,
	server::CacheRef,
	connection::Connection,
	frame::Frame,
	parse::Parse,
	object::Object,
	command::{Command, hash},
};

pub struct Set {
	key: u64,
	value: Object,
	ttl: Option<u32>,
}

impl Command for Set {
	fn parse_frames(parse: &mut Parse) -> Result<Self, ServerError> {
		let key = parse.next_bytes()?;
		let hashed_key = hash(&key);
		let object = Object::new(parse.next_bytes()?);

		let ttl = match parse.next_u32()? {
			0 => None,
			ttl => Some(ttl),
		};

		let command = Set {
			key: hashed_key,
			value: object,
			ttl,
		};

		Ok(command)
	}

	async fn apply(self, dst: &mut Connection, cache: &CacheRef) -> Result<(), ServerError> {
		cache.set(self.key, self.value, self.ttl)?;

		let frame = Frame::Array(vec![Frame::Bool(true)]);
		dst.write_frame(&frame).await?;

		Ok(())
	}
}
