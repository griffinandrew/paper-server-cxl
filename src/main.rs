/*
 * Copyright (c) Kia Shakiba
 *
 * This source code is licensed under the GNU AGPLv3 license found in the
 * LICENSE file in the root directory of this source tree.
 */

mod command;
mod config;
mod connection;
mod error;
mod logo;
mod server;

use std::{
	path::{Path, PathBuf},
	process,
	sync::Arc,
};

use clap::Parser;
use dotenv::dotenv;
use log::{error, info, warn};

use crate::{
	config::Config,
	server::{Server, new_cache},
};

#[cfg(all(feature = "tiered", feature = "all_dram"))]
compile_error!(
	"the `tiered` and `all_dram` features serve different caches; build one \
	 of them (`--no-default-features --features all_dram` for the flat \
	 baseline)"
);

// No `#[global_allocator]` here on purpose. `paper-cache` installs a
// NUMA-bound jemalloc of its own, and that allocator is what decides which
// node a page lands on -- the entire subject of the tiered cache. Declaring a
// second one is a hard error, and upstream's plain jemalloc would defeat the
// measurement even if it were not.

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

			Err(err) => fatal(err),
		},

		None => Config::default(),
	};

	if let Err(err) = config.validate() {
		fatal(err);
	}

	if !config.policies().is_empty() {
		warn!("policies[] is ignored: this server runs the one policy it is configured with");
	}

	let cache = match new_cache(&config) {
		Ok(cache) => cache,
		Err(err) => fatal(format!("could not construct the cache: {err}")),
	};

	let cache_version = cache.version();

	let server = match Server::new(&config, cache) {
		Ok(server) => {
			logo::print(&cache_version, config.port());
			Arc::new(server)
		},

		Err(err) => fatal(err),
	};

	init_ctrlc(server.clone());

	loop {
		if server.listen().is_ok() {
			info!("Shutting down server...");
			break;
		}
	}
}

/// A startup failure: said once, and the exit status says so too.
fn fatal(message: impl std::fmt::Display) -> ! {
	error!("{message}");
	process::exit(1);
}

fn init_logging<P>(maybe_path: Option<P>)
where
	P: AsRef<Path>,
{
	match maybe_path {
		Some(path) => {
			log4rs::init_file(path, Default::default()).expect("Could not initialize log4rs");
		},

		None => {
			let config_str = std::include_str!("../log4rs.yaml");
			let config = serde_yaml::from_str::<log4rs::config::RawConfig>(config_str)
				.expect("Invalid log config");

			log4rs::init_raw_config(config).expect("Could not initialize log4rs");
		},
	}
}

fn init_ctrlc(server: Arc<Server>) {
	let result = ctrlc::set_handler(move || {
		let _ = server.shutdown();
	});

	if result.is_err() {
		error!("Could not initailize ctrl-c handler");
	}
}
