mod logo;
mod server_error;
mod command;
mod tcp_server;
mod tcp_connection;
mod config;

use std::sync::{Arc};
use tokio::sync::Mutex;
use clap::Parser;
use paper_core::error::PaperError;
use paper_cache::{PaperCache, SizeOfObject};
use crate::tcp_server::TcpServer;
use crate::logo::ASCII_LOGO;
use crate::config::Config;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
	#[arg(short, long, default_value = "127.0.0.1")]
	host: String,

	#[arg(short, long, default_value_t = 3145)]
	port: u32,

	#[arg(short, long)]
	config: String,
}

#[tokio::main]
async fn main() {
	let args = Args::parse();

	let config = match Config::from_file(&args.config) {
		Ok(config) => config,

		Err(err) => {
			println!("\x1B[31mErr\x1B[0m: {}", err.message());
			return;
		},
	};

	let size_of_object: SizeOfObject<String> = |data: &String| {
		data.len() as u64
	};

	let cache = Arc::new(Mutex::new(
		PaperCache::<u32, String>::new(
			*config.get_max_size(),
			size_of_object,
			None
		).unwrap()
	));

	let mut server = match TcpServer::new(&args.host, &args.port, cache).await {
		Ok(server) => {
			println!("{}", ASCII_LOGO);
			println!("\x1B[36mListening for connections...\x1B[0m");

			server
		},

		Err(err) => {
			println!("\x1B[31mErr\x1B[0m: {}", err.message());
			return;
		},
	};

	loop {
		if let Err(err) = server.listen().await {
			println!("\x1B[31mErr\x1B[0m: {}", err.message());
			continue;
		}
	}
}
