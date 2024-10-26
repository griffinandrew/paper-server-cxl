mod logo;
mod error;
mod command;
mod server;
mod connection;
mod config;

use std::path::PathBuf;
use clap::Parser;
use dotenv::dotenv;
use log::error;
use paper_cache::PaperCache;
use paper_utils::stream::Buffer;

#[cfg(not(target_env = "msvc"))]
use tikv_jemallocator::Jemalloc;

use crate::{
	server::{Server, NoHasher},
	config::Config,
};

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
	#[arg(short, long)]
	config: Option<PathBuf>,
}

fn main() {
	dotenv().ok();
	init_logging();

	let args = Args::parse();

	let config = match &args.config {
		Some(path) => match Config::from_file(path) {
			Ok(config) => config,

			Err(err) => {
				error!("{err}");
				return;
			},
		},

		None => Config::default(),
	};

	let cache = PaperCache::<u64, Buffer, NoHasher>::with_hasher(
		config.max_size(),
		config.policy(),
		NoHasher::default(),
	).expect("Could not configure cache.");

	let cache_version = cache.version();

	let mut server = match Server::new(&config, cache) {
		Ok(server) => {
			logo::print(&cache_version, config.port());
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

fn init_logging() {
	let config_str = std::include_str!("../log4rs.yaml");
	let config = serde_yml::from_str::<log4rs::config::RawConfig>(config_str)
		.expect("Invalid log config.");

	log4rs::init_raw_config(config).unwrap();
}
