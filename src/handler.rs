use tokio::sync::mpsc;

use crate::{
	error::ServerError,
	vault::Vault,
	shutdown::Shutdown,
	connection::Connection,
	command::CommandType,
};

pub struct Handler {
	pub vault: Vault,

	pub connection: Connection,

	pub shutdown: Shutdown,
	pub _shutdown_complete: mpsc::Sender<()>,
}

impl Handler {
	pub async fn run(&mut self) -> Result<(), ServerError> {
		while !self.shutdown.is_shutdown() {
			let maybe_frame = tokio::select! {
				res = self.connection.read_frame() => res?,
				_ = self.shutdown.recv() => return Ok(()),
			};

			let frame = match maybe_frame {
				Some(frame) => frame,
				None => return Ok(()),
			};

			match CommandType::from_frame(frame)? {
				CommandType::Auth(auth_command) => {
					auth_command.authenticate(&mut self.connection, &mut self.vault).await?;
				},

				command => {
					let cache = match self.vault.cache() {
						Ok(cache) => cache,

						Err(err) => {
							self.connection.write_frame(&err.into_frame()).await?;
							continue;
						},
					};

					command.apply(&mut self.connection, cache).await?;
				},
			}
		}

		Ok(())
	}
}
