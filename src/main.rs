mod error;
mod command;
mod tcp_server;

use std::io;
use crate::tcp_server::TcpServer;

#[tokio::main]
async fn main() {
	let mut server = match TcpServer::new("127.0.0.1", &3145).await {
		Ok(server) => {
			println!("Listening for connections...");

			server
		},

		Err(err) => {
			println!("\x1B[31mErr\x1B[0m: {}", err.message());
			return;
		},
	};

	loop {
		if let Err(err) = server.get_command().await {
			println!("\x1B[31mErr\x1B[0m: {}", err.message());
			continue;
		}
	}
}
