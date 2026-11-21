//! People v1 state: the user's profile, contacts, "other contacts" and the Workspace directory.

pub mod routes;

use gmail::people::Person;

/// Google's generated avatar, which every person without a photo gets.
pub const DEFAULT_PHOTO: &str = "https://lh3.googleusercontent.com/a/default-user=s100";

/// Everyone the People API knows about.
#[derive(Debug, Clone, Default)]
pub struct Directory {
    pub me: Person,
    pub contacts: Vec<Person>,
    pub other_contacts: Vec<Person>,
    pub directory: Vec<Person>,
    /// Bumped on changes; sync tokens carry it.
    pub revision: u64,
}
