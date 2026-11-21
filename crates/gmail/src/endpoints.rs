//! Where each Google service lives. Paths are fixed by the APIs; only origins are configurable,
//! which is what lets Exo switch between Google and the mock by swapping this one value.

use url::Url;

/// Origins of every Google service the client talks to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    /// Gmail v1 (`/gmail/v1/...`, `/upload/gmail/v1/...`, `/batch/gmail/v1`).
    pub gmail: Url,
    /// Calendar v3 (`/calendar/v3/...`).
    pub calendar: Url,
    /// People v1 (`/v1/people/...`, `/v1/otherContacts...`).
    pub people: Url,
    /// OAuth 2.0 endpoints, which live on three different hosts.
    pub oauth: OAuthEndpoints,
}

/// Full URLs of Google's OAuth 2.0 endpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthEndpoints {
    /// Where the user is sent to consent (`/o/oauth2/v2/auth`).
    pub authorization: Url,
    /// Code exchange and refresh (`/token`).
    pub token: Url,
    /// The signed-in user's profile (`/oauth2/v2/userinfo`).
    pub userinfo: Url,
    /// Revokes a grant (`/revoke`).
    pub revocation: Url,
}

impl Endpoints {
    /// Google's production endpoints.
    #[must_use]
    pub fn google() -> Self {
        Self {
            gmail: parse("https://gmail.googleapis.com/"),
            calendar: parse("https://www.googleapis.com/"),
            people: parse("https://people.googleapis.com/"),
            oauth: OAuthEndpoints {
                authorization: parse("https://accounts.google.com/o/oauth2/v2/auth"),
                token: parse("https://oauth2.googleapis.com/token"),
                userinfo: parse("https://www.googleapis.com/oauth2/v2/userinfo"),
                revocation: parse("https://oauth2.googleapis.com/revoke"),
            },
        }
    }

    /// Every service served from one origin, the way `gmail-mock` serves them.
    #[must_use]
    pub fn mock(origin: &Url) -> Self {
        let at = |path: &str| {
            let mut url = origin.clone();
            url.set_path(path);
            url
        };
        Self {
            gmail: at("/"),
            calendar: at("/"),
            people: at("/"),
            oauth: OAuthEndpoints {
                authorization: at("/o/oauth2/v2/auth"),
                token: at("/token"),
                userinfo: at("/oauth2/v2/userinfo"),
                revocation: at("/revoke"),
            },
        }
    }
}

impl Default for Endpoints {
    fn default() -> Self {
        Self::google()
    }
}

fn parse(url: &str) -> Url {
    Url::parse(url).expect("hard-coded Google URLs are valid")
}

/// `origin` with `segments` appended as percent-encoded path segments.
pub(crate) fn join<'a>(origin: &Url, segments: impl IntoIterator<Item = &'a str>) -> Url {
    let mut url = origin.clone();
    url.path_segments_mut()
        .expect("http(s) URLs can have paths")
        .pop_if_empty()
        .extend(segments);
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_serves_everything_from_one_origin() {
        let origin = Url::parse("http://127.0.0.1:8787").unwrap();
        let endpoints = Endpoints::mock(&origin);
        assert_eq!(endpoints.gmail.as_str(), "http://127.0.0.1:8787/");
        assert_eq!(
            endpoints.oauth.token.as_str(),
            "http://127.0.0.1:8787/token"
        );
    }

    #[test]
    fn join_encodes_segments() {
        let url = join(
            &Endpoints::google().gmail,
            ["gmail", "v1", "users", "me", "a b"],
        );
        assert_eq!(
            url.as_str(),
            "https://gmail.googleapis.com/gmail/v1/users/me/a%20b"
        );
    }
}
