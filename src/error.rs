use std::io;
use thiserror::Error;
use paper_cache::CacheError;
use crate::frame::Frame;

#[derive(Debug, PartialEq, Error)]
pub enum ServerError {
	#[error(transparent)]
	CacheError(#[from] paper_cache::CacheError),

	#[error("an internal error occurred")]
	Internal,

	#[error("could not establish a connection")]
	InvalidAddress,

	#[error("could not establish a connection")]
	InvalidConnection,

	#[error("the maximum number of connections was exceeded")]
	MaxConnectionsExceeded,

	#[error("disconnected from client")]
	Disconnected,

	#[error("could not open config file")]
	InvalidConfig,

	#[error("invalid config line <{0}>")]
	InvalidConfigLine(String),

	#[error("invalid {0} config")]
	InvalidConfigParam(&'static str),

	#[error("invalid policy <{0}> in config")]
	InvalidConfigPolicy(String),

	#[error("unauthorized")]
	Unauthorized,
}

#[derive(Debug, PartialEq, Error)]
pub enum FrameError {
	#[error(transparent)]
	Server(#[from] ServerError),

	#[error("incomplete frame")]
	Incomplete,
}

#[derive(Debug, PartialEq, Error)]
pub enum ParseError {
	#[error(transparent)]
	Server(#[from] ServerError),

	#[error("end of stream")]
	EndOfStream,

	#[error("invalid protocol")]
	InvalidProtocol,
}

impl From<io::Error> for ServerError {
	fn from(_: io::Error) -> Self {
		ServerError::Internal
	}
}

impl From<FrameError> for ServerError {
	fn from(value: FrameError) -> Self {
		match value {
			FrameError::Server(err) => err,
			_ => ServerError::Internal,
		}
	}
}

impl From<ParseError> for ServerError {
	fn from(value: ParseError) -> Self {
		match value {
			ParseError::Server(err) => err,
			_ => ServerError::Internal,
		}
	}
}

impl ServerError {
	pub fn into_frame(self) -> Frame {
		if let ServerError::CacheError(err) = &self {
			let mut frames = vec![Frame::Bool(false)];

			frames.push(Frame::Byte(get_error_code(&self)));
			frames.push(Frame::Byte(get_cache_error_code(err)));

			return Frame::Array(frames);
		}

		let mut frames = vec![Frame::Bool(false)];
		frames.push(Frame::Byte(get_error_code(&self)));
		Frame::Array(frames)
	}
}

fn get_error_code(error: &ServerError) -> u8 {
	match error {
		ServerError::CacheError(_)					=> 0,

		ServerError::Internal
			| ServerError::InvalidAddress
			| ServerError::InvalidConnection
			| ServerError::Disconnected
			| ServerError::InvalidConfig
			| ServerError::InvalidConfigLine(_)
			| ServerError::InvalidConfigParam(_)
			| ServerError::InvalidConfigPolicy(_)	=> 1,

		ServerError::MaxConnectionsExceeded			=> 2,
		ServerError::Unauthorized					=> 3,
	}
}

fn get_cache_error_code(error: &CacheError) -> u8 {
	match error {
		CacheError::Internal			=> 0,

		CacheError::KeyNotFound			=> 1,

		CacheError::ZeroValueSize		=> 2,
		CacheError::ExceedingValueSize	=> 3,

		CacheError::ZeroCacheSize		=> 4,
	}
}
