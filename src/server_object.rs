use kwik::mem;
use paper_core::stream::Buffer;
use paper_cache::ObjectMemSize;

#[derive(Clone)]
pub struct ServerObject(Buffer);

impl ServerObject {
	pub fn new(buf: Buffer) -> Self {
		ServerObject(buf)
	}

	pub fn into_buf(self) -> Buffer {
		self.0
	}
}

impl ObjectMemSize for ServerObject {
	fn mem_size(&self) -> usize {
		mem::size_of_vec(&self.0)
	}
}
