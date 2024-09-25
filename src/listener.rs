use std::sync::Arc;
use log::error;

use tokio::{
	net::{TcpListener, TcpStream},
	sync::{Semaphore, broadcast, mpsc},
	time::{self, Duration},
};

use crate::{
	error::ServerError,
	vault::Vault,
	handler::Handler,
	shutdown::Shutdown,
	connection::Connection,
};

const MAX_BACKOFF: u64 = 64;

pub struct Listener {
	pub vault: Vault,

	pub listener: TcpListener,
	pub limit_connections: Arc<Semaphore>,

	pub notify_shutdown: broadcast::Sender<()>,
	pub shutdown_complete_tx: mpsc::Sender<()>,
}

impl Listener {
	pub async fn run(&mut self) -> Result<(), ServerError> {
		loop {
			let permit = self.limit_connections
				.clone()
				.acquire_owned().await
				.unwrap();

			let socket = self.accept().await?;

			let mut handler = Handler {
				vault: self.vault.clone(),

				connection: Connection::new(socket)?,

				shutdown: Shutdown::new(self.notify_shutdown.subscribe()),
				_shutdown_complete: self.shutdown_complete_tx.clone(),
			};

			tokio::spawn(async move {
				if let Err(err) = handler.run().await {
					error!("{}", err);
				}

				drop(permit);
			});
		}
	}

	async fn accept(&mut self) -> Result<TcpStream, ServerError> {
		let mut backoff = 1;

		loop {
			match self.listener.accept().await {
				Ok((socket, _)) => return Ok(socket),

				Err(_) => {
					if backoff > MAX_BACKOFF {
						return Err(ServerError::InvalidConnection);
					}
				},
			}

			time::sleep(Duration::from_secs(backoff)).await;
			backoff *= 2;
		}
	}
}
