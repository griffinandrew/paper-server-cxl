use kwik::mem;
use paper_core::stream::Buffer;
use paper_cache::ObjectMemSize;

#[derive(Clone)]
pub struct ObjectBuffer {
	buf: Buffer,
}

impl ObjectBuffer {
	pub fn new(buf: Buffer) -> Self {
		ObjectBuffer {
			buf,
		}
	}

	pub fn to_buf(self) -> Buffer {
		self.buf
	}
}

impl ObjectMemSize for ObjectBuffer {
	fn mem_size(&self) -> usize {
		mem::size_of_vec(&self.buf)
	}
}
