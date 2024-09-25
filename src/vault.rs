use crate::{
	server::CacheRef,
	error::ServerError,
};

pub struct Vault {
	cache: CacheRef,
	auth_token: Option<u64>,
	locked: bool,
}

impl Vault {
	pub fn new(cache: CacheRef, auth_token: Option<u64>) -> Self {
		Vault {
			cache,
			auth_token,
			locked: auth_token.is_some(),
		}
	}

	pub fn cache(&self) -> Result<&CacheRef, ServerError> {
		if self.locked {
			return Err(ServerError::Unauthorized);
		}

		Ok(&self.cache)
	}

	pub fn try_unlock(&mut self, token: u64) -> Result<(), ServerError> {
		if !self.locked {
			return Ok(());
		}

		if self.auth_token.is_some_and(|auth_token| auth_token != token) {
			return Err(ServerError::Unauthorized);
		}

		self.locked = false;
		Ok(())
	}
}

impl Clone for Vault {
	fn clone(&self) -> Self {
		Vault::new(self.cache.clone(), self.auth_token)
	}
}
