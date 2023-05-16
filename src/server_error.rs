use std::error::Error;
use std::fmt::{Display, Formatter};
pub use paper_core::error::PaperError;

#[derive(PartialEq, Debug)]
pub enum ErrorKind {
	InvalidAddress,
	InvalidConnection,

	InvalidCommand,
	InvalidResponse,

	InvalidStream,

	ConnectionLost,
}

#[derive(Debug)]
pub struct ServerError {
	kind: ErrorKind,
	message: String,
}

impl ServerError {
	pub fn new(kind: ErrorKind, message: &str) -> Self {
		ServerError {
			kind,
			message: message.to_owned(),
		}
	}

	pub fn kind(&self) -> &ErrorKind {
		&self.kind
	}
}

impl PaperError for ServerError {
	fn message(&self) -> &str {
		&self.message
	}
}

impl Error for ServerError {}

impl Display for ServerError {
	fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
		write!(f, "{}", self.message)
	}
}
