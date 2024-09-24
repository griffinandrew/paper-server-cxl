use std::sync::Arc;
use log::error;

use tokio::{
	net::{TcpListener, TcpStream},
	sync::{Semaphore, broadcast, mpsc},
	time::{self, Duration},
};

use crate::{
	error::ServerError,
	server::CacheRef,
	handler::Handler,
	shutdown::Shutdown,
	connection::Connection,
};

const MAX_BACKOFF: u64 = 64;

pub struct Listener {
	pub cache: CacheRef,

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
				cache: self.cache.clone(),

				connection: Connection::new(socket),

				shutdown: Shutdown::new(self.notify_shutdown.subscribe()),
				shutdown_complete: self.shutdown_complete_tx.clone(),
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
