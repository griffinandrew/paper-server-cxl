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

pub struct Stats;

impl Command for Stats {
	fn parse_frames(_: &mut Parse) -> Result<Self, ServerError> {
		Ok(Stats)
	}

	async fn apply(self, dst: &mut Connection, cache: &CacheRef) -> Result<(), ServerError> {
		let stats = cache.stats();

		let policy_byte = match stats.get_policy() {
			PaperPolicy::Lfu => PolicyByte::LFU,
			PaperPolicy::Fifo => PolicyByte::FIFO,
			PaperPolicy::Lru => PolicyByte::LRU,
			PaperPolicy::Mru => PolicyByte::MRU,
		};

		let frames = vec![
			Frame::Bool(true),
			Frame::U64(stats.get_max_size()),
			Frame::U64(stats.get_used_size()),
			Frame::U64(stats.get_total_gets()),
			Frame::U64(stats.get_total_sets()),
			Frame::U64(stats.get_total_dels()),
			Frame::F64(stats.get_miss_ratio()),
			Frame::Byte(policy_byte),
			Frame::U64(stats.get_uptime()),
		];

		let frame = Frame::Array(frames);
		dst.write_frame(&frame).await?;

		Ok(())
	}
}
