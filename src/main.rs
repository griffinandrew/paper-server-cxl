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
mod selfstats;
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

	/// Address to listen on, overriding `host` and `port` of the config
	#[arg(long, value_name = "ADDR:PORT")]
	bind: Option<String>,

	/// Overall cache capacity, overriding `max_size`: a byte count, or with a
	/// suffix (2GiB) as in the config
	#[arg(long, value_name = "BYTES")]
	max_size: Option<String>,

	/// Fast (DRAM) tier capacity, overriding `fast_tier_size`; the tiered build
	/// only, and it must not exceed the overall capacity
	#[arg(long, value_name = "BYTES")]
	fast_tier_size: Option<String>,

	/// Eviction policy, overriding `policy`: a hybrid design such as
	/// lru-compact-hybrid or s3-fifo-faithful-compact-hybrid-0.1 (the all_dram
	/// build serves the flat designs instead)
	#[arg(long, value_name = "POLICY")]
	policy: Option<String>,

	/// Require this token via the AUTH command, overriding `auth_token`
	#[arg(long, value_name = "TOKEN")]
	auth: Option<String>,

	/// Print the server-side cache latency report to stderr every this many
	/// seconds. Command byte 200 returns the same report on demand.
	#[arg(long, value_name = "SECONDS")]
	stats_interval: Option<u64>,
}

fn main() {
	let args = Args::parse();

	dotenv().ok();
	init_logging(args.log_config.as_ref());

	let mut config = match &args.config {
		Some(path) => match Config::from_file(path) {
			Ok(config) => config,

			Err(err) => fatal(err),
		},

		None => Config::default(),
	};

	// The flags win over the file, which wins over default.pconf.
	if let Err(err) = apply_flags(&mut config, &args) {
		fatal(err);
	}

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

	info!(
		"Serving {} with max size {} B and fast tier {} B",
		config.policy(),
		config.max_size(),
		config.fast_tier_size(),
	);

	if let Some(every) = config.stats_interval() {
		server.spawn_stats_reporter(every);
	}

	init_ctrlc(server.clone());

	loop {
		if server.listen().is_ok() {
			info!("Shutting down server...");
			break;
		}
	}
}

/// The command line's settings, over the config's. Each is the config line it
/// stands for (`Config::set`), so a flag is parsed exactly as the file's line
/// would be, and a bad one names its flag.
fn apply_flags(config: &mut Config, args: &Args) -> Result<(), String> {
	let flagged = |flag: &str, result: Result<(), error::ServerError>| {
		result.map_err(|err| format!("{flag}: {err}"))
	};

	if let Some(bind) = &args.bind {
		flagged("--bind", config.set_bind(bind))?;
	}

	if let Some(value) = &args.max_size {
		flagged("--max-size", config.set("max_size", value))?;
	}

	if let Some(value) = &args.fast_tier_size {
		flagged("--fast-tier-size", config.set("fast_tier_size", value))?;
	}

	if let Some(value) = &args.policy {
		flagged("--policy", config.set("policy", value))?;
	}

	if let Some(value) = &args.auth {
		flagged("--auth", config.set("auth_token", value))?;
	}

	if let Some(seconds) = args.stats_interval {
		flagged("--stats-interval", config.set_stats_interval(seconds))?;
	}

	Ok(())
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
