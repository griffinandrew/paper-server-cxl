mod logo;
mod server_error;
mod command;
mod tcp_server;
mod tcp_connection;
mod config;
mod server_object;

use std::sync::{Arc, Mutex};
use clap::Parser;
use paper_utils::error::PaperError;
use paper_cache::PaperCache;

use crate::{
    tcp_server::TcpServer,
    logo::ASCII_LOGO,
    config::Config,
    server_object::ServerObject,
};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
	#[arg(short, long)]
	config: String,
}

fn main() {
	let args = Args::parse();

	let config = match Config::from_file(&args.config) {
		Ok(config) => config,

		Err(err) => {
			println!("\x1B[31mErr\x1B[0m: {}", err.message());
			return;
		},
	};

	let cache = Arc::new(Mutex::new(
		PaperCache::<u32, ServerObject>::new(
			config.max_size(),
			Some(config.policies().to_vec()),
		).unwrap()
	));

	let mut server = match TcpServer::new(config.host(), config.port(), cache) {
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
		if let Err(err) = server.listen() {
			println!("\x1B[31mErr\x1B[0m: {}", err.message());
			continue;
		}
	}
}
