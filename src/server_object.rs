use paper_utils::stream::Buffer;
use paper_cache::{ObjectMemSize, ObjectSize};

#[derive(Clone)]
pub struct ServerObject(Buffer);

impl ServerObject {
	pub fn new(buf: Buffer) -> Self {
		ServerObject(buf)
	}

	pub fn as_buf(&self) -> &Buffer {
		&self.0
	}
}

impl ObjectMemSize for ServerObject {
	fn mem_size(&self) -> ObjectSize {
		self.0.len() as ObjectSize
	}
}
