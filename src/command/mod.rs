mod ping;
mod version;
mod auth;
mod get;
mod set;
mod del;
mod has;
mod peek;
mod ttl;
mod size;
mod wipe;
mod resize;
mod policy;
mod stats;

use std::hash::{DefaultHasher, Hash, Hasher};
use bytes::Bytes;
use paper_utils::command::CommandByte;

use crate::{
	error::ServerError,
	server::CacheRef,
	connection::Connection,
	frame::Frame,
	parse::Parse,
	command::{
		ping::Ping,
		version::Version,
		auth::Auth,
		get::Get,
		set::Set,
		del::Del,
		has::Has,
		peek::Peek,
		ttl::Ttl,
		size::Size,
		wipe::Wipe,
		resize::Resize,
		policy::Policy,
		stats::Stats,
	},
};

pub enum CommandType {
	Ping(Ping),
	Version(Version),

	//Auth(Bytes),

	Get(Get),
	Set(Set),
	Del(Del),

	Has(Has),
	Peek(Peek),
	Ttl(Ttl),
	Size(Size),

	Wipe(Wipe),

	Resize(Resize),
	Policy(Policy),

	Stats(Stats),
/*




*/
}

trait Command {
	fn parse_frames(parse: &mut Parse) -> Result<Self, ServerError>
	where
		Self: Sized,
	;

	async fn apply(self, dst: &mut Connection, cache: &CacheRef) -> Result<(), ServerError>;
}

impl CommandType {
	pub fn from_frame(frame: Frame) -> Result<Self, ServerError> {
		let mut parse = Parse::new(frame)?;
		let command_byte = parse.next_byte()?;

		let command = match command_byte {
			CommandByte::PING => CommandType::Ping(Ping::parse_frames(&mut parse)?),
			CommandByte::VERSION => CommandType::Version(Version::parse_frames(&mut parse)?),

			CommandByte::GET => CommandType::Get(Get::parse_frames(&mut parse)?),
			CommandByte::SET => CommandType::Set(Set::parse_frames(&mut parse)?),
			CommandByte::DEL => CommandType::Del(Del::parse_frames(&mut parse)?),

			CommandByte::HAS => CommandType::Has(Has::parse_frames(&mut parse)?),
			CommandByte::PEEK => CommandType::Peek(Peek::parse_frames(&mut parse)?),
			CommandByte::TTL => CommandType::Ttl(Ttl::parse_frames(&mut parse)?),
			CommandByte::SIZE => CommandType::Size(Size::parse_frames(&mut parse)?),

			CommandByte::WIPE => CommandType::Wipe(Wipe::parse_frames(&mut parse)?),

			CommandByte::RESIZE => CommandType::Resize(Resize::parse_frames(&mut parse)?),
			CommandByte::POLICY => CommandType::Policy(Policy::parse_frames(&mut parse)?),

			CommandByte::STATS => CommandType::Stats(Stats::parse_frames(&mut parse)?),

			_ => return Err(ServerError::Internal),
		};

		parse.finish()?;

		Ok(command)
	}

	pub async fn apply(
		self,
		dst: &mut Connection,
		cache: &CacheRef,
	) -> Result<(), ServerError> {
		let res = match self {
			CommandType::Ping(command) => command.apply(dst, cache).await,
			CommandType::Version(command) => command.apply(dst, cache).await,

			CommandType::Get(command) => command.apply(dst, cache).await,
			CommandType::Set(command) => command.apply(dst, cache).await,
			CommandType::Del(command) => command.apply(dst, cache).await,

			CommandType::Has(command) => command.apply(dst, cache).await,
			CommandType::Peek(command) => command.apply(dst, cache).await,
			CommandType::Ttl(command) => command.apply(dst, cache).await,
			CommandType::Size(command) => command.apply(dst, cache).await,

			CommandType::Wipe(command) => command.apply(dst, cache).await,

			CommandType::Resize(command) => command.apply(dst, cache).await,
			CommandType::Policy(command) => command.apply(dst, cache).await,

			CommandType::Stats(command) => command.apply(dst, cache).await,
		};

		if let Err(err) = res {
			dst.write_frame(&err.into_frame()).await?;
		}

		Ok(())
	}
}

fn hash(key: &Bytes) -> u64 {
	let mut s = DefaultHasher::new();
	key.hash(&mut s);
	s.finish()
}
