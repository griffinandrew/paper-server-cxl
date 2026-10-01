/*
 * Copyright (c) Kia Shakiba
 *
 * This source code is licensed under the GNU AGPLv3 license found in the
 * LICENSE file in the root directory of this source tree.
 */

//! The SET arm over a real socket: a value admitted before it is read, refused
//! without being buffered, and abandoned when its client goes quiet.
//!
//! Each test starts the server binary (`CARGO_BIN_EXE_paper-server`) on a port
//! of its own and talks to it in frames built by hand here, sharing nothing with
//! the server's own code. The server is killed, by its PID, when the test ends.
//! Nothing here is a load: a few connections and a few MiB. Set
//! `SET_PATH_TEST_LOGS` to a directory to keep each server's output there, one
//! file per port.

#![cfg(feature = "tiered")]

use std::{
	fs::File,
	io::{Read, Write},
	net::{TcpListener, TcpStream},
	process::{self, Child, Command, Stdio},
	sync::atomic::{AtomicU16, Ordering},
	thread,
	time::{Duration, Instant},
};

const TRUE: u8 = 33;
const FALSE: u8 = 63;

const GET: u8 = 3;
const SET: u8 = 4;
const SELF_STATS: u8 = 200;
const PING: u8 = 0;
const AUTH: u8 = 2;
const STATUS: u8 = 13;

struct Server {
	child: Child,
	port: u16,
}

/// A port no other test in this run is using, and none that is bound right now.
/// The tests run in parallel, and a port asked of the OS and released before the
/// server binds it can be handed to the next test too: two servers, one port,
/// and a test talking to the other's. So ports are dealt from a base of this
/// process's own, one apiece, below the range the kernel picks the source
/// ports of outgoing connections from (32768 and up).
fn free_port() -> u16 {
	static NEXT: AtomicU16 = AtomicU16::new(0);

	let base = 20_000 + (process::id() % 600) as u16 * 16;

	loop {
		let port = base + NEXT.fetch_add(1, Ordering::Relaxed) % 16;

		if TcpListener::bind(("127.0.0.1", port)).is_ok() {
			return port;
		}
	}
}

impl Server {
	/// Starts the server and waits until it is listening AND is the child just
	/// started: another process can take the port between `free_port` and the
	/// server's bind, and answer in its place. A server that is not ours, or
	/// that never comes up, is killed and the start is tried again on another
	/// port; a few failures in a row are a real problem and fail the test.
	fn start(args: &[&str], env: &[(&str, &str)]) -> Server {
		let mut why = Vec::new();

		for _ in 0..5 {
			let mut server = Server::spawn(free_port(), args, env);

			match server.wait_until_ours(args) {
				Ok(()) => return server,
				Err(reason) => {
					// Visible with --nocapture; a test that needed a retry is a
					// flake in the making, and the reason is what to look at.
					eprintln!("server start: port {}: {reason}", server.port);
					why.push(format!("port {}: {reason}", server.port));
				},
			}
		}

		panic!("the server did not come up in five tries: {why:?}");
	}

	fn spawn(port: u16, args: &[&str], env: &[(&str, &str)]) -> Server {
		// The server's log is dropped, unless SET_PATH_TEST_LOGS names a
		// directory to keep one per port in.
		let log = |name: &str| match std::env::var_os("SET_PATH_TEST_LOGS") {
			Some(dir) => Stdio::from(File::create(std::path::Path::new(&dir).join(format!("{port}.{name}"))).unwrap()),
			None => Stdio::null(),
		};

		let child = Command::new(env!("CARGO_BIN_EXE_paper-server"))
			.args(["--bind", &format!("127.0.0.1:{port}")])
			.args(args)
			.envs(env.iter().copied())
			.stdin(Stdio::null())
			.stdout(log("out"))
			.stderr(log("err"))
			.spawn()
			.expect("the server binary starts");

		Server { child, port }
	}

	fn wait_until_ours(&mut self, args: &[&str]) -> Result<(), String> {
		let started = Instant::now();

		while started.elapsed() < Duration::from_secs(15) {
			if let Some(status) = self.child.try_wait().unwrap() {
				return Err(format!("the server exited at once ({status}): was the port taken?"));
			}

			if let Ok(stream) = TcpStream::connect(("127.0.0.1", self.port)) {
				match self.pid_of(stream, args) {
					Ok(pid) if pid == self.child.id() => return Ok(()),
					Ok(pid) => return Err(format!("the port answers as pid {pid}, not {}", self.child.id())),

					// Something is listening and is not (yet) a server that
					// answers: try again until the time is up.
					Err(_) => {},
				}
			}

			thread::sleep(Duration::from_millis(20));
		}

		Err("nothing that was our server answered in 15 s".to_string())
	}

	/// The pid in the server's STATUS reply. Errors, instead of panicking, so a
	/// listener that is not a server only costs a retry.
	fn pid_of(&self, mut stream: TcpStream, args: &[&str]) -> std::io::Result<u32> {
		stream.set_read_timeout(Some(Duration::from_secs(5)))?;

		let mut byte = [0u8; 1];
		let said_true = |byte: u8, what: &str| match byte {
			TRUE => Ok(()),
			other => Err(std::io::Error::other(format!("{what}: got {other}"))),
		};

		stream.read_exact(&mut byte)?;
		said_true(byte[0], "the handshake")?;

		// STATUS needs the token, when the server asks for one.
		if let Some(at) = args.iter().position(|arg| *arg == "--auth") {
			stream.write_all(&[&[AUTH][..], &buf(args[at + 1].as_bytes())].concat())?;
			stream.read_exact(&mut byte)?;
			said_true(byte[0], "AUTH")?;
		}

		stream.write_all(&[STATUS])?;
		stream.read_exact(&mut byte)?;
		said_true(byte[0], "STATUS")?;

		let mut pid = [0u8; 4];

		stream.read_exact(&mut pid)?;

		Ok(u32::from_le_bytes(pid))
	}

	fn client(&self) -> Client {
		Client::connect(self.port)
	}
}

impl Drop for Server {
	fn drop(&mut self) {
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}

/// A reply that was not a success: the first code byte, and the cache code that
/// follows when that is 0.
#[derive(Debug, PartialEq)]
struct Refusal {
	server: u8,
	cache: Option<u8>,
}

struct Client {
	stream: TcpStream,
}

impl Client {
	fn connect(port: u16) -> Client {
		Client::from_stream(TcpStream::connect(("127.0.0.1", port)).unwrap())
	}

	fn from_stream(stream: TcpStream) -> Client {
		stream.set_nodelay(true).unwrap();
		stream.set_read_timeout(Some(Duration::from_secs(20))).unwrap();

		let mut client = Client { stream };

		assert_eq!(client.byte(), TRUE, "the server speaks first, with a true");

		client
	}

	fn byte(&mut self) -> u8 {
		let mut b = [0u8; 1];

		self.stream.read_exact(&mut b).unwrap();

		b[0]
	}

	fn u32(&mut self) -> u32 {
		let mut b = [0u8; 4];

		self.stream.read_exact(&mut b).unwrap();

		u32::from_le_bytes(b)
	}

	fn buf(&mut self) -> Vec<u8> {
		let len = self.u32() as usize;
		let mut b = vec![0u8; len];

		self.stream.read_exact(&mut b).unwrap();

		b
	}

	/// `Ok` on a true reply; the refusal otherwise.
	fn ok(&mut self) -> Result<(), Refusal> {
		match self.byte() {
			TRUE => Ok(()),

			FALSE => match self.byte() {
				0 => Err(Refusal { server: 0, cache: Some(self.byte()) }),
				server => Err(Refusal { server, cache: None }),
			},

			other => panic!("not a boolean indicator: {other}"),
		}
	}

	fn send(&mut self, bytes: &[u8]) {
		self.stream.write_all(bytes).unwrap();
	}

	fn ping(&mut self) -> Vec<u8> {
		self.send(&[PING]);
		self.ok().expect("a PING is answered");

		self.buf()
	}

	fn set(&mut self, key: &[u8], value: &[u8]) -> Result<(), Refusal> {
		self.send(&set_frame(key, value));

		self.ok()
	}

	fn get(&mut self, key: &[u8]) -> Result<Vec<u8>, Refusal> {
		self.send(&[&[GET][..], &buf(key)].concat());
		self.ok()?;

		Ok(self.buf())
	}

	fn auth(&mut self, token: &[u8]) {
		self.send(&[&[AUTH][..], &buf(token)].concat());
		self.ok().expect("the token is right");
	}

	fn report(&mut self) -> Report {
		self.send(&[SELF_STATS]);
		self.ok().expect("the report is answered");

		Report(String::from_utf8(self.buf()).unwrap())
	}

	/// Whether the server closes this connection within `within`.
	fn closed_within(&mut self, within: Duration) -> bool {
		self.stream.set_read_timeout(Some(within)).unwrap();

		let mut b = [0u8; 1];

		match self.stream.read(&mut b) {
			Ok(0) => true,
			Ok(_) => false,
			Err(err) => err.kind() == std::io::ErrorKind::ConnectionReset,
		}
	}
}

fn buf(bytes: &[u8]) -> Vec<u8> {
	[&(bytes.len() as u32).to_le_bytes()[..], bytes].concat()
}

fn set_frame(key: &[u8], value: &[u8]) -> Vec<u8> {
	[&[SET][..], &buf(key), &buf(value), &0u32.to_le_bytes()].concat()
}

/// The first `len` bytes of a SET frame whose value is `value_len` bytes: its
/// key, its length and a prefix of the value, and no more.
fn partial_set(key: &[u8], value: &[u8], sent: usize) -> Vec<u8> {
	[&[SET][..], &buf(key)[..], &(value.len() as u32).to_le_bytes(), &value[..sent]].concat()
}

struct Report(String);

impl Report {
	/// The number after `label` on the line that starts with it.
	fn figure(&self, label: &str) -> u64 {
		let line = self
			.0
			.lines()
			.find(|line| line.starts_with(label))
			.unwrap_or_else(|| panic!("no line starts with {label:?} in\n{}", self.0));

		line[label.len()..]
			.trim_start()
			.split(|c: char| !c.is_ascii_digit())
			.next()
			.unwrap()
			.parse()
			.unwrap()
	}
}

fn eventually(within: Duration, mut condition: impl FnMut() -> bool) -> bool {
	let started = Instant::now();

	loop {
		if condition() {
			return true;
		}

		if started.elapsed() >= within {
			return false;
		}

		thread::sleep(Duration::from_millis(25));
	}
}

fn value(len: usize, seed: u8) -> Vec<u8> {
	(0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
}

#[test]
fn a_value_is_read_into_the_cache_and_comes_back_byte_exact() {
	let server = Server::start(&["--max-size", "64MiB", "--fast-tier-size", "16MiB"], &[]);
	let mut client = server.client();

	// Keys are bytes: not UTF-8, with a NUL inside.
	for (key, len) in [(&b"\xff\xfe\x00key"[..], 1), (b"plain", 4096), (b"large", 1_572_864)] {
		let expected = value(len, len as u8);

		client.set(key, &expected).unwrap();
		assert_eq!(client.get(key).unwrap(), expected, "{len} B");
	}

	// The set went through a permit: the report counts it, and no setter is
	// left live once the SET is over.
	let report = client.report();

	assert_eq!(report.figure("permit sets"), 3);
	assert_eq!(report.figure("live setters"), 0);
}

#[test]
fn a_value_the_cache_cannot_hold_is_skipped_and_the_connection_stays_in_step() {
	let server = Server::start(&["--max-size", "1MiB", "--fast-tier-size", "512KiB"], &[]);
	let mut client = server.client();

	let too_big = value(3 << 20, 7);

	// Cache error 3, ExceedingValueSize: the cache would not hold it.
	assert_eq!(client.set(b"big", &too_big), Err(Refusal { server: 0, cache: Some(3) }));

	// The value was consumed and not stored: the next command parses.
	assert_eq!(client.ping(), b"pong");
	assert_eq!(client.get(b"big"), Err(Refusal { server: 0, cache: Some(1) }));

	let report = client.report();

	assert_eq!(report.figure("bytes skipped"), too_big.len() as u64);
	assert_eq!(report.figure("refused"), 1);
	assert_eq!(report.figure("objects"), 0);
}

#[test]
fn a_client_that_stalls_mid_value_is_closed_and_its_bytes_come_back() {
	let server = Server::start(
		&["--max-size", "64MiB", "--fast-tier-size", "32MiB", "--set-timeout", "600"],
		&[],
	);
	let mut control = server.client();
	let before = control.report().figure("phys fast");

	// A value of 1 MiB, half sent, then nothing.
	let whole = value(1 << 20, 1);
	let mut stalled = server.client();

	stalled.send(&partial_set(b"stalled", &whole, whole.len() / 2));

	// Its allocation is charged to the fast tier while it waits for the rest...
	assert!(
		eventually(Duration::from_secs(3), || control.report().figure("phys fast") >= before + whole.len() as u64),
		"the stalled SET pins no fast bytes",
	);
	assert_eq!(control.report().figure("live setters"), 1);

	// ...and the server gives up on it, closes the connection and refunds them.
	assert!(stalled.closed_within(Duration::from_secs(5)), "the server did not close the stalled client");
	assert!(
		eventually(Duration::from_secs(3), || control.report().figure("phys fast") == before),
		"the fast-tier byte count did not return to {before}",
	);

	let report = control.report();

	assert_eq!(report.figure("body timeouts"), 1);
	assert_eq!(report.figure("live setters"), 0);

	// Nothing was stored, and the server serves on.
	assert_eq!(control.get(b"stalled"), Err(Refusal { server: 0, cache: Some(1) }));
	control.set(b"after", b"fine").unwrap();
	assert_eq!(control.get(b"after").unwrap(), b"fine");
}

#[test]
fn a_set_the_full_fast_tier_cannot_take_is_code_8_and_the_connection_stays_in_step() {
	let server = Server::start(
		&["--max-size", "256MiB", "--fast-tier-size", "16MiB", "--set-timeout", "4000"],
		&[("PAPER_GATE_STALL_WINDOW_MS", "100")],
	);
	let mut control = server.client();
	let eff = control.report().figure("eff fast cap");
	let before = control.report().figure("phys fast");

	// Four SETs of a quarter of the budget each, stalled before the last byte of
	// their values: each is admitted (the tier settles below its target until
	// the fourth), and together they pin the whole budget, where nothing can
	// demote them -- they are not yet in the cache.
	let hold = (eff / 4) as usize;
	let held = value(hold, 3);
	let mut stalled: Vec<Client> = Vec::new();

	for i in 0..4 {
		let mut client = server.client();

		client.send(&partial_set(format!("held-{i}").as_bytes(), &held, hold - 1));
		stalled.push(client);
	}

	assert!(
		eventually(Duration::from_secs(5), || control.report().figure("phys fast") >= before + 4 * hold as u64),
		"the four stalled SETs did not pin the tier",
	);
	assert_eq!(control.report().figure("live setters"), 4);

	// A fifth SET waits at the byte gate for the stall window, and is refused.
	let mut client = server.client();
	let refused_at = Instant::now();

	assert_eq!(
		client.set(b"refused", &value(hold * 2, 9)),
		Err(Refusal { server: 0, cache: Some(8) }),
	);
	// It is the gate's stall window that ends the wait, not the set timeout.
	assert!(refused_at.elapsed() < Duration::from_millis(2000), "{:?}", refused_at.elapsed());

	// The refused SET's value was skipped, so the next command on the same
	// connection parses -- also when it is already queued behind the value.
	assert_eq!(client.ping(), b"pong");

	client.send(&[&set_frame(b"refused-again", &value(hold, 5))[..], &[PING]].concat());
	assert_eq!(client.ok(), Err(Refusal { server: 0, cache: Some(8) }));
	client.ok().expect("the PING behind it is answered");
	assert_eq!(client.buf(), b"pong");

	let report = control.report();

	assert_eq!(report.figure("refused"), 2);
	assert!(report.0.contains("2 fast tier stalled [8], 0 metadata overflow [9]"), "{}", report.0);

	// The stalled clients are closed once their reads time out, and the tier
	// is whole again.
	for client in &mut stalled {
		assert!(client.closed_within(Duration::from_secs(8)));
	}

	assert!(
		eventually(Duration::from_secs(3), || control.report().figure("phys fast") == before),
		"the fast-tier byte count did not return to {before}",
	);
	assert_eq!(control.report().figure("body timeouts"), 4);

	// And the SET that was refused is admitted now.
	let big = value(hold * 2, 9);

	client.set(b"refused", &big).unwrap();
	assert_eq!(client.get(b"refused").unwrap(), big);
}

#[test]
fn a_new_key_with_no_room_for_its_metadata_is_code_9_and_the_connection_stays_in_step() {
	// The metadata floor is the whole fast tier, so no new key's metadata fits.
	let server = Server::start(
		&["--max-size", "64MiB", "--fast-tier-size", "1MiB", "--set-timeout", "1000"],
		&[("PAPER_GATE_METADATA_FLOOR_BYTES", "1048576")],
	);
	let mut client = server.client();

	assert_eq!(client.set(b"new", &value(1000, 1)), Err(Refusal { server: 0, cache: Some(9) }));
	assert_eq!(client.ping(), b"pong");

	client.send(&[&set_frame(b"newer", &value(70_000, 2))[..], &[PING]].concat());
	assert_eq!(client.ok(), Err(Refusal { server: 0, cache: Some(9) }));
	client.ok().unwrap();
	assert_eq!(client.buf(), b"pong");

	let report = client.report();

	assert!(report.0.contains("0 fast tier stalled [8], 2 metadata overflow [9]"), "{}", report.0);
	assert_eq!(report.figure("bytes skipped"), 71_000);
}

#[test]
fn an_unauthorized_set_is_consumed_and_refused_with_server_error_3() {
	let server = Server::start(&["--max-size", "64MiB", "--fast-tier-size", "16MiB", "--auth", "tok"], &[]);
	let mut anonymous = server.client();

	// A 2 MiB value from a client that has not authorized: read past, so the
	// PING queued behind it is the next frame.
	anonymous.send(&[&set_frame(b"anon", &value(2 << 20, 4))[..], &[PING]].concat());
	assert_eq!(anonymous.ok(), Err(Refusal { server: 3, cache: None }));
	anonymous.ok().unwrap();
	assert_eq!(anonymous.buf(), b"pong");

	let mut client = server.client();

	client.auth(b"tok");
	assert_eq!(client.get(b"anon"), Err(Refusal { server: 0, cache: Some(1) }));
}
