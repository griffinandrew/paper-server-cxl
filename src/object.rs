use bytes::Bytes;
use typesize::TypeSize;

pub struct Object(Bytes);

impl Object {
	pub fn new(data: Bytes) -> Self {
		Object(data)
	}

	pub fn data(&self) -> Bytes {
		self.0.clone()
	}
}

impl TypeSize for Object {
	fn extra_size(&self) -> usize {
		self.0.len()
	}
}
