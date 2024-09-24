use std::{
	sync::Arc,
	future::Future,
	hash::BuildHasherDefault,
};

use tokio::{
	net::TcpListener,
	sync::{Semaphore, broadcast, mpsc},
};

use nohash_hasher::NoHashHasher;
use paper_cache::PaperCache;

use crate::{
	config::Config,
	error::ServerError,
	object::Object,
	listener::Listener,
};

type NoHasher = BuildHasherDefault<NoHashHasher<u64>>;
pub type CacheRef = Arc<PaperCache<u64, Object, NoHasher>>;

pub struct Server {
	cache: CacheRef,

	max_connections: usize,
	tcp_listener: TcpListener,
}

impl Server {
	pub async fn init(config: &Config) -> Result<Self, ServerError> {
		let addr = format!("{}:{}", config.host(), config.port());

		let cache = PaperCache::<u64, Object, NoHasher>::with_hasher(
			config.max_size(),
			config.policies(),
			NoHasher::default(),
		).expect("Could not configure cache.");

		let tcp_listener = TcpListener::bind(addr).await
			.map_err(|_| ServerError::InvalidAddress)?;

		let server = Server {
			cache: Arc::new(cache),

			max_connections: config.max_connections(),
			tcp_listener,
		};

		Ok(server)
	}

	pub async fn listen(self, shutdown: impl Future) {
		let (notify_shutdown, _) = broadcast::channel(1);
		let (shutdown_complete_tx, mut shutdown_complete_rx) = mpsc::channel(1);

		let mut server = Listener {
			cache: self.cache.clone(),

			listener: self.tcp_listener,
			limit_connections: Arc::new(Semaphore::new(self.max_connections)),

			notify_shutdown,
			shutdown_complete_tx,
		};

		tokio::select! {
			res = server.run() => {
			},

			_ = shutdown => {
			},
		}

		let Listener {
			shutdown_complete_tx,
			notify_shutdown,
			..
		} = server;

		drop(notify_shutdown);
		drop(shutdown_complete_tx);

		let _ = shutdown_complete_rx.recv().await;
	}
}
