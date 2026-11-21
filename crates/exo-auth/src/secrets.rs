//! Where refresh tokens live: the macOS Keychain, one item per account.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::Error;

/// Keeps one secret (a refresh token) per account address.
///
/// Calls are synchronous and may block: the Keychain can ask the user to allow access (macOS
/// asks again whenever an unsigned build changes), so keep them off the UI thread.
pub trait SecretStore: Send + Sync + 'static {
    /// The secret for `account`, if one is stored.
    ///
    /// # Errors
    /// [`Error::Secrets`] when the store can't be read.
    fn get(&self, account: &str) -> Result<Option<String>, Error>;

    /// Stores `secret` for `account`, replacing any earlier one.
    ///
    /// # Errors
    /// [`Error::Secrets`] when the store can't be written.
    fn set(&self, account: &str, secret: &str) -> Result<(), Error>;

    /// Forgets `account`'s secret. Forgetting one that isn't there succeeds.
    ///
    /// # Errors
    /// [`Error::Secrets`] when the store can't be written.
    fn delete(&self, account: &str) -> Result<(), Error>;
}

/// Lets a caller keep a handle on the store it hands over.
impl<S: SecretStore + ?Sized> SecretStore for Arc<S> {
    fn get(&self, account: &str) -> Result<Option<String>, Error> {
        (**self).get(account)
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), Error> {
        (**self).set(account, secret)
    }

    fn delete(&self, account: &str) -> Result<(), Error> {
        (**self).delete(account)
    }
}

/// Secrets in memory only, for tests and gmail-mock sessions.
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<String, String>>);

/// Leaves the secrets out.
impl std::fmt::Debug for MemorySecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("MemorySecrets").finish_non_exhaustive()
    }
}

impl MemorySecrets {
    fn secrets(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.0.lock().expect("secret map is never poisoned")
    }
}

impl SecretStore for MemorySecrets {
    fn get(&self, account: &str) -> Result<Option<String>, Error> {
        Ok(self.secrets().get(account).cloned())
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), Error> {
        self.secrets().insert(account.to_owned(), secret.to_owned());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), Error> {
        self.secrets().remove(account);
        Ok(())
    }
}

/// The login keychain: a generic password per account, under the service `Exo Google sign-in`
/// with the account's address as its account name (as Keychain Access shows them).
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, Default)]
pub struct Keychain;

#[cfg(target_os = "macos")]
mod keychain {
    use security_framework::passwords::{
        PasswordOptions, delete_generic_password, generic_password, set_generic_password,
    };

    use super::{Keychain, SecretStore};
    use crate::Error;

    const SERVICE: &str = "Exo Google sign-in";
    /// `errSecItemNotFound`.
    const NOT_FOUND: i32 = -25300;

    fn failed(error: impl std::error::Error + Send + Sync + 'static) -> Error {
        Error::Secrets(Box::new(error))
    }

    /// The item's text. The error leaves the bytes out (`FromUtf8Error` would list them).
    fn text(secret: Vec<u8>) -> Result<String, Error> {
        String::from_utf8(secret).map_err(|error| failed(error.utf8_error()))
    }

    impl SecretStore for Keychain {
        fn get(&self, account: &str) -> Result<Option<String>, Error> {
            match generic_password(PasswordOptions::new_generic_password(SERVICE, account)) {
                Ok(secret) => text(secret).map(Some),
                Err(error) if error.code() == NOT_FOUND => Ok(None),
                Err(error) => Err(failed(error)),
            }
        }

        fn set(&self, account: &str, secret: &str) -> Result<(), Error> {
            set_generic_password(SERVICE, account, secret.as_bytes()).map_err(failed)
        }

        fn delete(&self, account: &str) -> Result<(), Error> {
            match delete_generic_password(SERVICE, account) {
                Err(error) if error.code() != NOT_FOUND => Err(failed(error)),
                _ => Ok(()),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_garbled_item_stays_out_of_errors() {
            // `FromUtf8Error`'s `Debug` lists its bytes: `[49, 47, 47, …]` is `1//`.
            let error = text(b"1//secret\xff".to_vec()).unwrap_err();
            assert!(
                !format!("{error} {error:?}").contains("49, 47, 47"),
                "{error:?}"
            );
        }

        #[test]
        #[ignore = "writes to the login keychain"]
        fn round_trips_through_the_login_keychain() {
            let account = "exo-auth-test@example.invalid";
            Keychain.delete(account).unwrap();
            assert_eq!(Keychain.get(account).unwrap(), None);
            Keychain.set(account, "1//first").unwrap();
            Keychain.set(account, "1//second").unwrap();
            assert_eq!(Keychain.get(account).unwrap().as_deref(), Some("1//second"));
            Keychain.delete(account).unwrap();
            assert_eq!(Keychain.get(account).unwrap(), None);
            Keychain.delete(account).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_leaves_secrets_out() {
        let secrets = MemorySecrets::default();
        secrets.set("maya@brightmoor.co", "1//secret").unwrap();
        assert!(!format!("{secrets:?}").contains("1//secret"));
    }
}
