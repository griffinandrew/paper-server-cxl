/*
 * Copyright (c) Griffin Andrew
 *
 * This source code is licensed under the GNU AGPLv3 license found in the
 * LICENSE file in the root directory of this source tree.
 */

//! The buffer a connection reads every request's key into.
//!
//! The protocol's byte strings were read one request at a time into a `Box<[u8]>`
//! of their own (`paper_utils::stream::read_buf`: `vec![0; len]`, then
//! `read_exact`), so a request allocated, zero-filled and freed a buffer for its
//! key. The cache takes a key as `&[u8]` (`PaperCache::get_borrowed` and its
//! kin, `reserve_set_borrowed`), so the key is read into this buffer, which the
//! connection keeps, and lent to the cache from there: in the steady state a
//! request allocates nothing for its key, and writes it once, off the socket.
//!
//! The buffer is zero-filled only when it has to GROW (a key longer than any it
//! has read), never to read a key it already has room for. One that grew past
//! [`RETAINED`] is given back after the request that needed it, so a client that
//! sends one huge key does not leave its connection holding the space for as long
//! as it stays open.

use std::io::{self, Read};

/// The most a connection keeps of its key buffer between requests.
pub const RETAINED: usize = 64 * 1024;

#[derive(Default)]
pub struct KeyBuf {
	/// Initialized from the start to its length: `read_exact` reads into a slice
	/// of it, so nothing is zero-filled but the growth.
	buf: Vec<u8>,

	/// The length of the key last read: the front of `buf`.
	len: usize,
}

impl KeyBuf {
	/// Reads exactly `len` bytes off `reader` into the buffer, which becomes
	/// the key. On an error the key is empty.
	pub fn read_from(&mut self, reader: &mut impl Read, len: usize) -> io::Result<()> {
		self.len = 0;

		if self.buf.len() < len {
			self.buf.resize(len, 0);
		}

		reader.read_exact(&mut self.buf[..len])?;

		self.len = len;

		Ok(())
	}

	/// The key last read.
	pub fn bytes(&self) -> &[u8] {
		&self.buf[..self.len]
	}

	/// Gives back what the buffer holds beyond [`RETAINED`], once a request is
	/// done with its key.
	pub fn trim(&mut self) {
		if self.buf.len() > RETAINED {
			self.buf = Vec::new();
			self.len = 0;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_key_is_read_into_the_buffer_and_the_buffer_is_reused() {
		let mut buf = KeyBuf::default();

		buf.read_from(&mut &b"abcdef"[..], 6).unwrap();
		assert_eq!(buf.bytes(), b"abcdef");

		let ptr = buf.bytes().as_ptr();

		// A shorter key, and one of the same length, read into the same
		// allocation: nothing is allocated, and nothing of the last key shows.
		buf.read_from(&mut &b"xy"[..], 2).unwrap();
		assert_eq!(buf.bytes(), b"xy");
		assert_eq!(buf.bytes().as_ptr(), ptr);

		buf.read_from(&mut &b"123456"[..], 6).unwrap();
		assert_eq!(buf.bytes(), b"123456");
		assert_eq!(buf.bytes().as_ptr(), ptr);
	}

	#[test]
	fn an_empty_key_is_an_empty_key() {
		let mut buf = KeyBuf::default();

		buf.read_from(&mut &b""[..], 0).unwrap();
		assert_eq!(buf.bytes(), b"");
	}

	#[test]
	fn a_key_that_does_not_arrive_whole_is_an_error_and_leaves_no_key() {
		let mut buf = KeyBuf::default();

		buf.read_from(&mut &b"abc"[..], 3).unwrap();
		assert!(buf.read_from(&mut &b"ab"[..], 3).is_err());
		assert_eq!(buf.bytes(), b"");
	}

	#[test]
	fn a_buffer_that_grew_past_the_limit_is_given_back_and_a_small_one_is_kept() {
		let mut buf = KeyBuf::default();
		let big = vec![7u8; RETAINED + 1];

		buf.read_from(&mut &big[..], big.len()).unwrap();
		assert_eq!(buf.bytes().len(), RETAINED + 1);

		buf.trim();
		assert_eq!(buf.buf.capacity(), 0);

		buf.read_from(&mut &b"abc"[..], 3).unwrap();

		let ptr = buf.bytes().as_ptr();

		buf.trim();
		assert_eq!(buf.bytes(), b"abc");
		assert_eq!(buf.bytes().as_ptr(), ptr);
	}
}
