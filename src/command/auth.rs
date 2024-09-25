use crate::{
	error::ServerError,
	server::CacheRef,
	vault::Vault,
	connection::Connection,
	frame::Frame,
	parse::Parse,
	command::{Command, hash},
};

pub struct Auth {
	token: u64,
}

impl Command for Auth {
	fn parse_frames(parse: &mut Parse) -> Result<Self, ServerError> {
		let token = parse.next_bytes()?;
		let hashed = hash(&token);

		let command = Auth {
			token: hashed,
		};

		Ok(command)
	}

	async fn apply(self, _: &mut Connection, _: &CacheRef) -> Result<(), ServerError> {
		Err(ServerError::Internal)
	}
}

impl Auth {
	pub async fn authenticate(self, dst: &mut Connection, vault: &mut Vault) -> Result<(), ServerError> {
		match vault.try_unlock(self.token) {
			Ok(()) => {
				let frame = Frame::Array(vec![Frame::Bool(true)]);
				dst.write_frame(&frame).await?;
			},

			Err(err) => {
				dst.write_frame(&err.into_frame()).await?;
			},
		}

		Ok(())
	}
}
