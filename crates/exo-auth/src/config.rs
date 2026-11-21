//! Exo's OAuth client: Google's downloaded client JSON for a "Desktop app" client.
//!
//! Exo ships no client of its own. Everyone creates theirs in Google Cloud Console and puts the
//! JSON where [`ClientConfig::load`] looks.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use gmail::Endpoints;
use gmail::oauth::OAuthClient;
use serde::Deserialize;

use crate::Error;

/// The variable naming a client file, which wins over the config folder's.
pub(crate) const ENV: &str = "EXO_GOOGLE_OAUTH_CLIENT";
/// The client file's name in Exo's config folder.
pub(crate) const FILE_NAME: &str = "google-oauth-client.json";

/// Who Exo is to Google, and where Google is.
#[derive(Clone)]
pub struct ClientConfig {
    client_id: String,
    /// Google issues Desktop clients a secret and expects it, though it isn't confidential.
    /// Still never printed.
    client_secret: String,
    endpoints: Endpoints,
}

/// Leaves the secret out.
impl std::fmt::Debug for ClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientConfig")
            .field("client_id", &self.client_id)
            .field("endpoints", &self.endpoints)
            .finish_non_exhaustive()
    }
}

/// The parts of Google's client JSON Exo reads.
#[derive(Deserialize)]
struct File {
    installed: Option<Installed>,
    web: Option<serde::de::IgnoredAny>,
}

/// Not its `auth_uri` and `token_uri`: those are always Google's, so a client file someone
/// shares can't send the browser or the code exchange elsewhere.
#[derive(Deserialize)]
struct Installed {
    client_id: String,
    client_secret: String,
}

impl ClientConfig {
    /// A client for `endpoints`, e.g. a gmail-mock's.
    pub fn new(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        endpoints: Endpoints,
    ) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            endpoints,
        }
    }

    /// Reads the client file named by `EXO_GOOGLE_OAUTH_CLIENT`, or else
    /// `google-oauth-client.json` in `config_dir` (Exo's config folder,
    /// `~/Library/Application Support/Exo` on macOS). It talks to Google's production endpoints,
    /// whatever URLs the file lists.
    ///
    /// # Errors
    /// [`Error::MissingClient`] (whose message says where to put the file) when there's no file,
    /// [`Error::InvalidClient`] when it can't be read or isn't a Desktop app client.
    pub fn load(config_dir: &Path) -> Result<Self, Error> {
        let (path, from_env) = locate(std::env::var_os(ENV), config_dir);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::MissingClient { path, from_env });
            }
            Err(error) => {
                return Err(Error::InvalidClient {
                    path,
                    reason: format!("can't be read: {error}"),
                });
            }
        };
        Self::parse(&text).map_err(|reason| Error::InvalidClient { path, reason })
    }

    /// Parses Google's client JSON, explaining what's wrong when it isn't a Desktop app client.
    fn parse(text: &str) -> Result<Self, String> {
        let file: File = serde_json::from_str(text)
            .map_err(|error| format!("isn't Google's client JSON: {error}"))?;
        let installed = match (file.installed, file.web) {
            (Some(installed), _) => installed,
            (None, Some(_)) => {
                return Err(
                    "is for a \"Web application\" client; Exo needs a \"Desktop app\" client"
                        .into(),
                );
            }
            (None, None) => return Err("has no \"installed\" (Desktop app) client".into()),
        };
        Ok(Self::new(
            installed.client_id,
            installed.client_secret,
            Endpoints::google(),
        ))
    }

    /// The client's ID, which Google shows on the consent screen's "app info".
    #[must_use]
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// The Google APIs this client talks to.
    #[must_use]
    pub fn endpoints(&self) -> &Endpoints {
        &self.endpoints
    }

    pub(crate) fn oauth(&self) -> OAuthClient {
        OAuthClient::new(
            self.endpoints.oauth.clone(),
            &self.client_id,
            Some(self.client_secret.clone()),
        )
    }
}

/// The client file to read, and whether the environment named it. An empty variable is unset.
fn locate(env: Option<OsString>, config_dir: &Path) -> (PathBuf, bool) {
    match env.filter(|path| !path.is_empty()) {
        Some(path) => (PathBuf::from(path), true),
        None => (config_dir.join(FILE_NAME), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of Google's download, with made-up values.
    const DESKTOP: &str = r#"{"installed":{"client_id":"123-abc.apps.googleusercontent.com","project_id":"exo-dev","auth_uri":"https://accounts.google.com/o/oauth2/auth","token_uri":"https://oauth2.googleapis.com/token","auth_provider_x509_cert_url":"https://www.googleapis.com/oauth2/v1/certs","client_secret":"GOCSPX-not-a-real-secret","redirect_uris":["http://localhost"]}}"#;

    #[test]
    fn reads_a_desktop_client() {
        let config = ClientConfig::parse(DESKTOP).unwrap();
        assert_eq!(config.client_id(), "123-abc.apps.googleusercontent.com");
        assert_eq!(config.client_secret, "GOCSPX-not-a-real-secret");
        assert_eq!(config.endpoints(), &Endpoints::google());
    }

    #[test]
    fn a_file_cant_send_sign_in_elsewhere() {
        // A client file someone shared, pointing the code exchange (with its verifier) at
        // their server and the browser at a local file.
        let shared = DESKTOP
            .replace(
                "https://oauth2.googleapis.com/token",
                "https://collector.example/token",
            )
            .replace(
                "https://accounts.google.com/o/oauth2/auth",
                "file:///Users/someone/Downloads/run.command",
            );
        let config = ClientConfig::parse(&shared).unwrap();
        assert_eq!(config.endpoints(), &Endpoints::google());
    }

    #[test]
    fn debug_leaves_the_secret_out() {
        let config = ClientConfig::parse(DESKTOP).unwrap();
        assert!(!format!("{config:?}").contains("GOCSPX"));
        assert!(!format!("{:?}", config.oauth()).contains("GOCSPX"));
    }

    #[test]
    fn explains_wrong_files() {
        let web = r#"{"web":{"client_id":"x","client_secret":"y"}}"#;
        assert!(
            ClientConfig::parse(web)
                .unwrap_err()
                .contains("\"Desktop app\"")
        );
        assert!(
            ClientConfig::parse("{}")
                .unwrap_err()
                .contains("no \"installed\"")
        );
        assert!(
            ClientConfig::parse("not json")
                .unwrap_err()
                .contains("isn't Google's client JSON")
        );
    }

    #[test]
    fn the_environment_wins_over_the_config_folder() {
        let dir = Path::new("/config");
        assert_eq!(
            locate(Some("/elsewhere/client.json".into()), dir),
            (PathBuf::from("/elsewhere/client.json"), true)
        );
        assert_eq!(locate(None, dir), (dir.join(FILE_NAME), false));
        assert_eq!(
            locate(Some(OsString::new()), dir),
            (dir.join(FILE_NAME), false)
        );
    }

    #[test]
    fn missing_files_say_where_to_put_one() {
        let dir = tempfile::tempdir().unwrap();
        let message = match ClientConfig::load(dir.path()) {
            // Unless the environment names a file, which a developer's shell might.
            Err(
                error @ Error::MissingClient {
                    from_env: false, ..
                },
            ) => error.to_string(),
            Err(Error::MissingClient { from_env: true, .. }) | Ok(_) => return,
            Err(error) => panic!("{error}"),
        };
        assert!(
            message.contains(&dir.path().join(FILE_NAME).display().to_string()),
            "{message}"
        );
        assert!(
            message.contains("Desktop app") && message.contains(ENV),
            "{message}"
        );
    }

    /// Parses the real client file `EXO_GOOGLE_OAUTH_CLIENT` names (or the config folder's), without
    /// printing it: `EXO_GOOGLE_OAUTH_CLIENT=… cargo test -p exo-auth -- --ignored real_client`.
    #[test]
    #[ignore = "reads a real client file"]
    fn real_client_file_parses() {
        let config =
            ClientConfig::load(Path::new("/nonexistent")).map_err(|error| error.to_string());
        let config = config.expect("the client file parses");
        assert!(config.client_id().ends_with(".apps.googleusercontent.com"));
        assert!(!config.client_secret.is_empty());
        assert_eq!(
            config.endpoints().oauth.token.host_str(),
            Some("oauth2.googleapis.com")
        );
    }
}
