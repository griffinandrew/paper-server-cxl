use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(PartialEq, Debug)]
pub enum ErrorKind {
	InvalidAddress,
	InvalidConnection,

	InvalidCommand,

	InvalidStream,
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

	pub fn message(&self) -> &String {
		&self.message
	}
}

impl Error for ServerError {}

impl Display for ServerError {
	fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
		write!(f, "{}", self.message)
	}
}
