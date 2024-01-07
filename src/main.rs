mod logo;
mod server_error;
mod command;
mod tcp_server;
mod tcp_connection;
mod config;
mod server_object;

use std::sync::{Arc, Mutex};
use clap::Parser;
use log::error;
use paper_utils::stream::Buffer;
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
	log4rs::init_file("log4rs.yaml", Default::default()).unwrap();

	let args = Args::parse();

	let config = match Config::from_file(&args.config) {
		Ok(config) => config,

		Err(err) => {
			error!("{err}");
			return;
		},
	};

	let cache = Arc::new(Mutex::new(
		PaperCache::<Buffer, ServerObject>::new(
			config.max_size(),
			Some(config.policies().to_vec()),
		).unwrap()
	));

	let mut server = match TcpServer::new(&config, cache) {
		Ok(server) => {
			println!("{ASCII_LOGO}");
			println!("\x1B[36mListening on port {}...\x1B[0m", config.port());

			server
		},

		Err(err) => {
			error!("{err}");
			return;
		},
	};

	loop {
		let _ = server.listen();
	}
}
