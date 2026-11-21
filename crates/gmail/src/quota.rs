//! Gmail's per-user quota and what each method costs, from Google's "Usage limits" page
//! (<https://developers.google.com/workspace/gmail/api/reference/quota>, limits as of
//! May 1, 2026). Projects that used the API between November 2025 and April 2026 keep their
//! older, larger quota; every project created since gets these.

/// Quota units one user may spend per minute ("Per minute per user per project").
pub const PER_USER_PER_MINUTE: u32 = 6_000;

/// The quota units Gmail charges for the request `method path?query`, where `path` is the
/// URL's path (`/gmail/v1/users/me/threads/abc`, `/upload/gmail/v1/users/me/messages/send`).
/// `None` for anything that isn't a Gmail call (Calendar, People and OAuth have their own
/// quotas) and for the batch endpoint, whose calls are charged one by one.
#[must_use]
pub fn units(method: &str, path: &str, query: Option<&str>) -> Option<u32> {
    cost(method, path, query).map(|(_, units)| units)
}

/// [`units`], with the API method charged for, as Google's reference names it
/// (`history.list`, `threads.get`); `other` for a write the table doesn't list.
#[must_use]
pub fn cost(method: &str, path: &str, query: Option<&str>) -> Option<(&'static str, u32)> {
    let path = path.strip_prefix("/upload").unwrap_or(path);
    let mut rest = path.strip_prefix("/gmail/v1/users/")?.split('/');
    let _user = rest.next()?;
    let segments: Vec<&str> = rest.collect();
    // `format` defaults to `full`, the dearer variant.
    let full = || {
        let format = query
            .into_iter()
            .flat_map(|query| url::form_urlencoded::parse(query.as_bytes()))
            .find(|(name, _)| name == "format");
        !format.is_some_and(|(_, value)| value == "minimal" || value == "metadata")
    };
    let read = |cheap: u32, dear: u32| if full() { dear } else { cheap };
    Some(match (method, segments.as_slice()) {
        ("GET", ["profile"]) => ("getProfile", 1),
        ("POST", ["watch"]) => ("watch", 100),
        ("POST", ["stop"]) => ("stop", 50),
        ("GET", ["history"]) => ("history.list", 2),

        ("GET", ["labels"]) => ("labels.list", 1),
        ("GET", ["labels", _]) => ("labels.get", 1),
        ("POST", ["labels"]) => ("labels.create", 5),
        ("PUT" | "PATCH", ["labels", _]) => ("labels.update", 5),
        ("DELETE", ["labels", _]) => ("labels.delete", 5),

        ("GET", ["threads"]) => ("threads.list", 10),
        ("GET", ["threads", _]) => ("threads.get", read(40, 120)),
        ("POST", ["threads", _, "modify"]) => ("threads.modify", 10),
        ("POST", ["threads", _, "untrash"]) => ("threads.untrash", 10),
        ("POST", ["threads", _, "trash"]) => ("threads.trash", 20),
        ("DELETE", ["threads", _]) => ("threads.delete", 20),

        ("GET", ["messages"]) => ("messages.list", 5),
        ("GET", ["messages", _]) => ("messages.get", read(20, 60)),
        ("GET", ["messages", _, "attachments", _]) => ("messages.attachments.get", 20),
        ("POST", ["messages", "batchModify"]) => ("messages.batchModify", 50),
        ("POST", ["messages", "batchDelete"]) => ("messages.batchDelete", 50),
        ("POST", ["messages", "send"]) => ("messages.send", 100),
        ("POST", ["messages"]) => ("messages.insert", 25),
        ("POST", ["messages", "import"]) => ("messages.import", 25),
        ("POST", ["messages", _, "modify"]) => ("messages.modify", 5),
        ("POST", ["messages", _, "untrash"]) => ("messages.untrash", 5),
        ("POST", ["messages", _, "trash"]) => ("messages.trash", 20),
        ("DELETE", ["messages", _]) => ("messages.delete", 10),

        ("GET", ["drafts"]) => ("drafts.list", 5),
        ("GET", ["drafts", _]) => ("drafts.get", read(20, 60)),
        ("POST", ["drafts"]) => ("drafts.create", 10),
        ("DELETE", ["drafts", _]) => ("drafts.delete", 10),
        ("PUT", ["drafts", _]) => ("drafts.update", 15),
        ("POST", ["drafts", "send"]) => ("drafts.send", 100),

        ("GET", ["settings", ..]) => ("settings.get", 1),
        ("PUT" | "PATCH" | "POST", ["settings", "sendAs", ..]) => ("settings.sendAs", 100),
        ("PUT", ["settings", "vacation"]) => ("settings.updateVacation", 5),
        ("POST" | "DELETE", ["settings", "filters", ..]) => ("settings.filters", 5),
        // Every other write the page lists costs 5 or 100; the dearer keeps a budget safe.
        _ => ("other", 100),
    })
}

#[cfg(test)]
mod tests {
    use super::units;

    #[test]
    fn reads_cost_more_in_full() {
        assert_eq!(
            units("GET", "/gmail/v1/users/me/threads/abc", None),
            Some(120)
        );
        assert_eq!(
            units("GET", "/gmail/v1/users/me/threads/abc", Some("format=full")),
            Some(120)
        );
        assert_eq!(
            units(
                "GET",
                "/gmail/v1/users/me/threads/abc",
                Some("format=metadata&metadataHeaders=From")
            ),
            Some(40)
        );
        assert_eq!(
            units(
                "GET",
                "/gmail/v1/users/me/messages/abc",
                Some("format=minimal")
            ),
            Some(20)
        );
        assert_eq!(
            units("GET", "/gmail/v1/users/me/messages/abc", Some("format=raw")),
            Some(60)
        );
        assert_eq!(
            units("GET", "/gmail/v1/users/me/messages/m/attachments/a", None),
            Some(20)
        );
    }

    #[test]
    fn a_call_is_named_by_its_method() {
        assert_eq!(
            super::cost(
                "GET",
                "/gmail/v1/users/me/history",
                Some("startHistoryId=1")
            ),
            Some(("history.list", 2))
        );
        assert_eq!(
            super::cost(
                "GET",
                "/gmail/v1/users/me/threads/abc",
                Some("format=metadata")
            ),
            Some(("threads.get", 40))
        );
        assert_eq!(
            super::cost("PUT", "/gmail/v1/users/me/drafts/d", None),
            Some(("drafts.update", 15))
        );
    }

    #[test]
    fn writes_lists_and_uploads() {
        assert_eq!(
            units("POST", "/gmail/v1/users/me/messages/batchModify", None),
            Some(50)
        );
        assert_eq!(
            units("POST", "/gmail/v1/users/me/threads/t/modify", None),
            Some(10)
        );
        assert_eq!(
            units("POST", "/gmail/v1/users/me/messages/m/modify", None),
            Some(5)
        );
        assert_eq!(
            units("GET", "/gmail/v1/users/me/threads", Some("q=in%3Ainbox")),
            Some(10)
        );
        assert_eq!(
            units(
                "GET",
                "/gmail/v1/users/me/history",
                Some("startHistoryId=1")
            ),
            Some(2)
        );
        assert_eq!(
            units(
                "POST",
                "/upload/gmail/v1/users/me/messages/send",
                Some("uploadType=multipart")
            ),
            Some(100)
        );
        assert_eq!(
            units(
                "PUT",
                "/upload/gmail/v1/users/me/drafts/d",
                Some("uploadType=multipart")
            ),
            Some(15)
        );
        assert_eq!(
            units("POST", "/gmail/v1/users/me/drafts/send", None),
            Some(100)
        );
        assert_eq!(
            units("GET", "/gmail/v1/users/me/settings/sendAs", None),
            Some(1)
        );
        assert_eq!(
            units("POST", "/gmail/v1/users/me/settings/filters", None),
            Some(5)
        );
        assert_eq!(
            units("GET", "/gmail/v1/users/me/settings/vacation", None),
            Some(1)
        );
        assert_eq!(
            units("PUT", "/gmail/v1/users/me/settings/vacation", None),
            Some(5)
        );
    }

    #[test]
    fn other_apis_are_not_gmails() {
        assert_eq!(
            units("GET", "/calendar/v3/users/me/calendarList", None),
            None
        );
        assert_eq!(units("GET", "/v1/people/me/connections", None), None);
        assert_eq!(units("POST", "/batch/gmail/v1", None), None);
    }
}
