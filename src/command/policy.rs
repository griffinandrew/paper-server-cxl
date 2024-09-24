use paper_cache::PaperPolicy;
use paper_utils::policy::PolicyByte;

use crate::{
	error::ServerError,
	server::CacheRef,
	connection::Connection,
	frame::Frame,
	parse::Parse,
	command::Command,
};

pub struct Policy {
	policy: PaperPolicy,
}

impl Command for Policy {
	fn parse_frames(parse: &mut Parse) -> Result<Self, ServerError> {
		let policy = match parse.next_byte()? {
			PolicyByte::LFU => PaperPolicy::Lfu,
			PolicyByte::FIFO => PaperPolicy::Fifo,
			PolicyByte::LRU => PaperPolicy::Lru,
			PolicyByte::MRU => PaperPolicy::Mru,

			_ => return Err(ServerError::Internal),
		};

		let command = Policy {
			policy,
		};

		Ok(command)
	}

	async fn apply(self, dst: &mut Connection, cache: &CacheRef) -> Result<(), ServerError> {
		cache.policy(self.policy)?;

		let frame = Frame::Array(vec![Frame::Bool(true)]);
		dst.write_frame(&frame).await?;

		Ok(())
	}
}
