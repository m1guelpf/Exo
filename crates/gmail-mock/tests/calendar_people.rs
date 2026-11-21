//! Calendar v3 and People v1 through the `gmail` client.

mod support;

use gmail::calendar::*;
use gmail::people::PeopleQuery;

fn at(time: &str) -> EventDateTime {
    EventDateTime {
        date_time: Some(time.into()),
        time_zone: Some("America/New_York".into()),
        ..EventDateTime::default()
    }
}

#[tokio::test]
async fn calendar_list_and_events() {
    let server = support::server().await;
    let client = support::client(&server);
    let calendar = client.calendar();

    let list = calendar
        .calendar_list(&CalendarListQuery::default())
        .await
        .unwrap();
    assert!(
        list.items
            .iter()
            .any(|c| c.primary == Some(true) && c.id.as_deref() == Some("maya@brightmoor.co"))
    );
    assert!(
        list.items.iter().all(|c| c.hidden != Some(true)),
        "hidden calendars need showHidden"
    );
    let all = calendar
        .calendar_list(&CalendarListQuery {
            show_hidden: true,
            ..CalendarListQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(all.items.len(), list.items.len() + 1);

    let week = EventsQuery {
        time_min: Some("2026-11-16T00:00:00-05:00".into()),
        time_max: Some("2026-11-23T00:00:00-05:00".into()),
        single_events: true,
        order_by: Some("startTime".into()),
        ..EventsQuery::default()
    };
    let events = calendar.events("primary", &week).await.unwrap();
    let summaries: Vec<&str> = events
        .items
        .iter()
        .filter_map(|e| e.summary.as_deref())
        .collect();
    assert!(
        summaries.contains(&"Weekly product sync")
            && summaries.contains(&"Calloway: Atlas feedback call"),
        "{summaries:?}"
    );
    let sync = events
        .items
        .iter()
        .find(|e| e.summary.as_deref() == Some("Weekly product sync"))
        .unwrap();
    assert!(sync.recurring_event_id.is_some() && sync.id.as_deref().unwrap().contains('_'));
    assert_eq!(
        sync.start.as_ref().unwrap().date_time.as_deref(),
        Some("2026-11-16T10:00:00-05:00")
    );
    let tz = jiff::tz::TimeZone::get("America/New_York").unwrap();
    let starts: Vec<_> = events
        .items
        .iter()
        .map(|e| e.start.as_ref().unwrap().timestamp(&tz).unwrap())
        .collect();
    assert!(starts.windows(2).all(|w| w[0] <= w[1]));
    assert!(events.next_sync_token.is_some());

    let masters = calendar
        .events(
            "primary",
            &EventsQuery {
                single_events: false,
                order_by: None,
                ..week.clone()
            },
        )
        .await
        .unwrap();
    let master = masters
        .items
        .iter()
        .find(|e| e.summary.as_deref() == Some("Weekly product sync"))
        .unwrap();
    assert_eq!(master.recurrence, ["RRULE:FREQ=WEEKLY;BYDAY=MO"]);
    let instances = calendar
        .instances(
            "primary",
            master.id.as_deref().unwrap(),
            Some("2026-11-01T00:00:00Z"),
            Some("2026-12-01T00:00:00Z"),
        )
        .await
        .unwrap();
    assert_eq!(instances.items.len(), 5, "Mondays in November 2026");

    let focus = calendar
        .events(
            "primary",
            &EventsQuery {
                event_types: vec!["focusTime".into()],
                ..week.clone()
            },
        )
        .await
        .unwrap();
    assert!(
        !focus.items.is_empty()
            && focus
                .items
                .iter()
                .all(|e| e.event_type.as_deref() == Some("focusTime"))
    );
    let bad_order = calendar
        .events(
            "primary",
            &EventsQuery {
                single_events: false,
                ..week.clone()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(bad_order.status(), Some(400));
    assert!(
        calendar
            .events("nobody@example.com", &week)
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn event_lifecycle_rsvp_sync_and_free_busy() {
    let server = support::server().await;
    let client = support::client(&server);
    let calendar = client.calendar();

    let sync_token = calendar
        .events("primary", &EventsQuery::default())
        .await
        .unwrap()
        .next_sync_token
        .unwrap();
    let event = Event {
        summary: Some("Atlas pricing review".into()),
        start: Some(at("2026-11-18T13:00:00-05:00")),
        end: Some(at("2026-11-18T13:30:00-05:00")),
        attendees: vec![
            EventAttendee {
                email: Some("maya@brightmoor.co".into()),
                response_status: Some("needsAction".into()),
                ..EventAttendee::default()
            },
            EventAttendee {
                email: Some("lena@brightmoor.co".into()),
                response_status: Some("needsAction".into()),
                ..EventAttendee::default()
            },
        ],
        ..Event::default()
    };
    let created = calendar
        .insert_event("primary", &event, Some("all"))
        .await
        .unwrap();
    let id = created.id.clone().unwrap();
    assert_eq!(created.status.as_deref(), Some("confirmed"));
    assert!(
        created
            .ical_uid
            .as_deref()
            .unwrap()
            .ends_with("@google.com")
    );
    assert_eq!(created.organizer.as_ref().unwrap().is_self, Some(true));
    assert!(
        calendar
            .insert_event(
                "primary",
                &Event {
                    summary: Some("No times".into()),
                    ..Event::default()
                },
                None
            )
            .await
            .is_err()
    );
    // An RSVP patches just the user's attendee entry.
    let rsvp = Event {
        attendees_omitted: Some(true),
        attendees: vec![EventAttendee {
            email: Some("maya@brightmoor.co".into()),
            response_status: Some("accepted".into()),
            ..EventAttendee::default()
        }],
        ..Event::default()
    };
    let patched = calendar
        .patch_event("primary", &id, &rsvp, Some("none"))
        .await
        .unwrap();
    assert_eq!(patched.attendees.len(), 2);
    assert_eq!(
        patched.attendees[0].response_status.as_deref(),
        Some("accepted")
    );
    assert_eq!(
        patched.attendees[1].response_status.as_deref(),
        Some("needsAction")
    );
    assert_eq!(patched.summary.as_deref(), Some("Atlas pricing review"));
    assert_eq!(calendar.event("primary", &id).await.unwrap(), patched);

    let changes = calendar
        .events(
            "primary",
            &EventsQuery {
                sync_token: Some(sync_token.clone()),
                ..EventsQuery::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(changes.items.len(), 1);
    assert_eq!(changes.items[0].id.as_deref(), Some(id.as_str()));

    let busy = calendar
        .free_busy(&FreeBusyRequest {
            time_min: Some("2026-11-18T00:00:00-05:00".into()),
            time_max: Some("2026-11-19T00:00:00-05:00".into()),
            items: vec![FreeBusyRequestItem {
                id: "primary".into(),
            }],
            ..FreeBusyRequest::default()
        })
        .await
        .unwrap();
    let periods = &busy.calendars["primary"].busy;
    assert!(
        periods
            .iter()
            .any(|p| p.start == "2026-11-18T18:00:00Z" && p.end == "2026-11-18T18:30:00Z"),
        "{periods:?}"
    );

    calendar
        .delete_event("primary", &id, Some("all"))
        .await
        .unwrap();
    let gone = calendar
        .delete_event("primary", &id, None)
        .await
        .unwrap_err();
    assert_eq!(
        gone.status(),
        Some(410),
        "deleting twice says the resource has been deleted"
    );
    assert_eq!(
        calendar
            .event("primary", &id)
            .await
            .unwrap()
            .status
            .as_deref(),
        Some("cancelled")
    );
    let deletion = calendar
        .events(
            "primary",
            &EventsQuery {
                sync_token: Some(sync_token),
                ..EventsQuery::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        deletion.items[0].status.as_deref(),
        Some("cancelled"),
        "sync includes deletions"
    );

    let imported = calendar
        .import_event(
            "primary",
            &Event {
                ical_uid: Some("invite-123@example.com".into()),
                ..event.clone()
            },
        )
        .await
        .unwrap();
    assert_eq!(imported.ical_uid.as_deref(), Some("invite-123@example.com"));
    let again = calendar
        .import_event(
            "primary",
            &Event {
                ical_uid: Some("invite-123@example.com".into()),
                summary: Some("Renamed".into()),
                ..event.clone()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        again.id, imported.id,
        "importing the same iCalUID updates the copy"
    );

    let expired = calendar
        .events(
            "primary",
            &EventsQuery {
                sync_token: Some("stale".into()),
                ..EventsQuery::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(expired.status(), Some(410));

    // A colleague's free/busy comes from the meetings they're in; an outsider's is unknown.
    let busy = calendar
        .free_busy(&FreeBusyRequest {
            time_min: Some("2026-11-17T00:00:00-05:00".into()),
            time_max: Some("2026-11-18T00:00:00-05:00".into()),
            items: vec![
                FreeBusyRequestItem {
                    id: "tom@brightmoor.co".into(),
                },
                FreeBusyRequestItem {
                    id: "someone@example.com".into(),
                },
            ],
            ..FreeBusyRequest::default()
        })
        .await
        .unwrap();
    assert!(
        !busy.calendars["tom@brightmoor.co"].busy.is_empty(),
        "Tom runs the design crit"
    );
    assert_eq!(
        busy.calendars["someone@example.com"].errors[0].reason,
        "notFound"
    );
    // A colleague's calendar lists those meetings, theirs marked `self`; an outsider's is gone.
    let day = EventsQuery {
        time_min: Some("2026-11-17T00:00:00-05:00".into()),
        time_max: Some("2026-11-18T00:00:00-05:00".into()),
        single_events: true,
        ..EventsQuery::default()
    };
    let toms = calendar.events("tom@brightmoor.co", &day).await.unwrap();
    assert!(
        toms.items
            .iter()
            .any(|event| event.summary.as_deref() == Some("Design crit")),
        "Tom runs the design crit"
    );
    let selves: Vec<Option<&str>> = toms
        .items
        .iter()
        .flat_map(|event| {
            event
                .attendees
                .iter()
                .filter(|attendee| attendee.is_self == Some(true))
                .map(|attendee| attendee.email.as_deref())
        })
        .collect();
    assert!(
        selves
            .iter()
            .all(|email| *email == Some("tom@brightmoor.co")),
        "{selves:?}"
    );
    let outsider = calendar
        .events("someone@example.com", &day)
        .await
        .unwrap_err();
    assert_eq!(outsider.status(), Some(404));

    // A client may choose a base32hex id, once.
    let chosen = Event {
        id: Some("e0chosen01".into()),
        ..event.clone()
    };
    let kept = calendar
        .insert_event("primary", &chosen, None)
        .await
        .unwrap();
    assert_eq!(kept.id.as_deref(), Some("e0chosen01"));
    assert!(
        calendar
            .insert_event("primary", &chosen, None)
            .await
            .is_err(),
        "a duplicate"
    );
    let invalid = Event {
        id: Some("Not-Base32".into()),
        ..event
    };
    assert!(
        calendar
            .insert_event("primary", &invalid, None)
            .await
            .is_err()
    );

    // A Meet request comes back as a call; clearing it takes the link away.
    let meet = Event {
        conference_data: Some(
            serde_json::json!({ "createRequest": { "requestId": "r1", "conferenceSolutionKey": { "type": "hangoutsMeet" } } }),
        ),
        ..Event::default()
    };
    let with_meet = calendar
        .patch_event("primary", "e0chosen01", &meet, None)
        .await
        .unwrap();
    let link = with_meet.hangout_link.clone().unwrap();
    assert!(link.starts_with("https://meet.google.com/"));
    assert_eq!(
        with_meet.conference_data.as_ref().unwrap()["entryPoints"][0]["uri"],
        link.as_str()
    );
    let cleared = Event {
        conference_data: Some(serde_json::Value::Null),
        ..Event::default()
    };
    let without = calendar
        .patch_event("primary", "e0chosen01", &cleared, None)
        .await
        .unwrap();
    assert_eq!(
        (without.conference_data, without.hangout_link),
        (None, None)
    );
}

#[tokio::test]
async fn people_profile_contacts_and_directory() {
    let server = support::server().await;
    let client = support::client(&server);
    let people = client.people();

    let me = people.me("names,photos").await.unwrap();
    assert_eq!(me.names[0].display_name.as_deref(), Some("Maya Okafor"));
    assert_eq!(me.photos[0].default, Some(true));
    assert!(
        me.email_addresses.is_empty(),
        "personFields limits the fields returned"
    );

    let fields = "emailAddresses,names,metadata";
    let mut contacts = Vec::new();
    let mut query = PeopleQuery {
        page_size: Some(5),
        request_sync_token: true,
        ..PeopleQuery::default()
    };
    let sync_token = loop {
        let page = people.connections(fields, &query).await.unwrap();
        assert_eq!(page.total_items, Some(page.total_people.unwrap()));
        contacts.extend(page.connections);
        match page.next_page_token {
            Some(token) => query.page_token = Some(token),
            None => {
                break page
                    .next_sync_token
                    .expect("last page carries the sync token");
            }
        }
    };
    assert!(contacts.len() > 10);
    assert!(contacts.iter().all(
        |c| c.resource_name.as_deref().unwrap().starts_with("people/c") && c.metadata.is_some()
    ));
    let changes = people
        .connections(
            fields,
            &PeopleQuery {
                sync_token: Some(sync_token),
                ..PeopleQuery::default()
            },
        )
        .await
        .unwrap();
    assert!(changes.connections.is_empty() && changes.next_sync_token.is_some());
    let expired = people
        .connections(
            fields,
            &PeopleQuery {
                sync_token: Some("stale".into()),
                ..PeopleQuery::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(expired.status(), Some(400));

    let other = people
        .other_contacts(fields, &PeopleQuery::default())
        .await
        .unwrap();
    assert!(other.other_contacts.iter().all(|c| {
        c.resource_name
            .as_deref()
            .unwrap()
            .starts_with("otherContacts/")
    }));
    assert_eq!(
        other.total_size,
        Some(u32::try_from(other.other_contacts.len()).unwrap())
    );
    let found = people
        .search_other_contacts("mei", "names,emailAddresses")
        .await
        .unwrap();
    assert_eq!(
        found.results[0].person.email_addresses[0].value.as_deref(),
        Some("mei.watanabe@hollisconf.org")
    );

    let search = people
        .search_contacts("rach", "names,emailAddresses")
        .await
        .unwrap();
    assert_eq!(search.results.len(), 1);
    assert!(
        people
            .search_contacts("zzz", "names")
            .await
            .unwrap()
            .results
            .is_empty()
    );

    let directory = people
        .directory(
            "names,emailAddresses,organizations",
            &PeopleQuery::default(),
        )
        .await
        .unwrap();
    assert!(directory.people.len() >= 12);
    assert!(directory.people.iter().all(|p| {
        p.email_addresses[0]
            .value
            .as_deref()
            .unwrap()
            .ends_with("@brightmoor.co")
    }));
    assert_eq!(
        people.me("nonsenseField").await.unwrap_err().status(),
        Some(400)
    );
}
