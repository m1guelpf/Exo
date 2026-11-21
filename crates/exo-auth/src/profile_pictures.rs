//! The accounts' Google profile photos as the app last fetched them: one file per account,
//! named by its address, in `profile-pictures/` in the state folder. The folder is device-wide,
//! so clearing an account's mail leaves it, and signing out deletes the account's.

use std::io;
use std::path::{Path, PathBuf};

use crate::Error;

/// The folder's name in the state folder.
const DIR: &str = "profile-pictures";

#[derive(Debug)]
pub(crate) struct ProfilePictures {
    dir: PathBuf,
}

impl ProfilePictures {
    pub(crate) fn new(state_dir: &Path) -> Self {
        Self {
            dir: state_dir.join(DIR),
        }
    }

    /// `email`'s photo, `None` without one.
    pub(crate) fn get(&self, email: &str) -> Result<Option<Vec<u8>>, Error> {
        let path = self.dir.join(email);
        match std::fs::read(&path) {
            Ok(photo) => Ok(Some(photo)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(Error::ProfilePicture { path, source }),
        }
    }

    /// Saves `photo` as `email`'s through a temporary file, so a crash never leaves half of
    /// one; `None` deletes it.
    pub(crate) fn set(&self, email: &str, photo: Option<&[u8]>) -> Result<(), Error> {
        let path = self.dir.join(email);
        let failed = |source| Error::ProfilePicture {
            path: path.clone(),
            source,
        };
        let Some(photo) = photo else {
            return match std::fs::remove_file(&path) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(failed(error)),
                _ => Ok(()),
            };
        };
        let partial = self.dir.join(format!("{email}.partial"));
        std::fs::create_dir_all(&self.dir).map_err(failed)?;
        std::fs::write(&partial, photo).map_err(failed)?;
        std::fs::rename(&partial, &path).map_err(failed)
    }
}
