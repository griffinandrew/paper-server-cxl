use tokio::sync::mpsc;

use crate::{
	error::ServerError,
	server::CacheRef,
	shutdown::Shutdown,
	connection::Connection,
	command::CommandType,
};

pub struct Handler {
	pub cache: CacheRef,

	pub connection: Connection,

	pub shutdown: Shutdown,
	pub shutdown_complete: mpsc::Sender<()>,
}

impl Handler {
	pub async fn run(&mut self) -> Result<(), ServerError> {
		while !self.shutdown.is_shutdown() {
			let maybe_frame = tokio::select! {
				res = self.connection.read_frame() => {
					res.map_err(|_| ServerError::Internal)?
				},

				_ = self.shutdown.recv() => {
					return Ok(());
				}
			};

			let frame = match maybe_frame {
				Some(frame) => frame,
				None => return Ok(()),
			};

			let command = CommandType::from_frame(frame)?;
			command.apply(&mut self.connection, &self.cache).await?;
		}

		Ok(())
	}
}
