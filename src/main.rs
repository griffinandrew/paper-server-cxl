/*
 * Copyright (c) Kia Shakiba
 *
 * This source code is licensed under the GNU AGPLv3 license found in the
 * LICENSE file in the root directory of this source tree.
 */

mod logo;
mod error;
mod command;
mod server;
mod connection;
mod config;

use std::path::{Path, PathBuf};
use clap::Parser;
use dotenv::dotenv;
use log::error;

//#[cfg(not(target_env = "msvc"))]
//use tikv_jemallocator::Jemalloc;

use crate::{
	server::{Server, Cache},
	config::Config,
};

mod allocator;
use allocator::UmfGlobal;

//#[cfg(not(target_env = "msvc"))]
//#[global_allocator]
//static GLOBAL: Jemalloc = Jemalloc;

#[global_allocator]
static GLOBAL: UmfGlobal = UmfGlobal;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
	/// Optional path to PaperConfig (pconf) file
	#[arg(short, long)]
	config: Option<PathBuf>,

	#[arg(short, long)]
	/// Optional path to log4rs config file
	log_config: Option<PathBuf>,
}

fn main() {
	let args = Args::parse();

	dotenv().ok();
	init_logging(args.log_config);

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

	let cache = Cache::new(
		config.max_size(),
		config.policies(),
		config.policy(),
	).expect("Could not configure cache");

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

fn init_logging<P>(maybe_path: Option<P>)
where
	P: AsRef<Path>,
{
	match maybe_path {
		Some(path) => {
			log4rs::init_file(path, Default::default())
				.expect("Could not initialize log4rs");
		},

		None => {
			let config_str = std::include_str!("../log4rs.yaml");
			let config = serde_yaml::from_str::<log4rs::config::RawConfig>(config_str)
				.expect("Invalid log config");

			log4rs::init_raw_config(config)
				.expect("Could not initialize log4rs");
		}
	}
}
