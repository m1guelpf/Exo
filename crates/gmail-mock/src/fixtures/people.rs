//! People API records derived from the cast.

use gmail::people::{
    EmailAddress, FieldMetadata, Name, Organization, Person, PersonMetadata, PhoneNumber, Photo,
    Source,
};

use super::cast::{self, Circle};
use super::compose::fnv;
use crate::people::{DEFAULT_PHOTO, Directory};

/// Everyone the People API knows. `photo` is the account's own photo (without Google's size
/// option), when it has one.
pub fn directory(google_id: &str, photo: Option<&str>) -> Directory {
    let mut profile = person(
        cast::me(),
        &format!("people/{google_id}"),
        "PROFILE",
        google_id,
    );
    if let Some(photo) = photo {
        // People's URLs ask for 100px (`=s100`); `default` is left out for a photo of one's own.
        let primary = &mut profile.photos[0];
        primary.url = Some(format!("{photo}=s100"));
        primary.default = None;
    }
    let mut directory = Directory {
        me: profile,
        revision: 1,
        ..Directory::default()
    };
    for who in cast::PEOPLE.iter().filter(|who| !who.is_me()) {
        let id = format!(
            "{}",
            1_000_000_000_000_000_000u64 + fnv(who.email) % 8_000_000_000_000_000_000
        );
        match who.circle {
            Circle::Me => {}
            Circle::Colleague => directory.directory.push(person(
                who,
                &format!("people/{id}"),
                "DOMAIN_PROFILE",
                &id,
            )),
            Circle::Work | Circle::Personal => {
                let contact = format!("c{id}");
                directory.contacts.push(person(
                    who,
                    &format!("people/{contact}"),
                    "CONTACT",
                    &contact,
                ));
            }
            Circle::Acquaintance => {
                let contact = format!("c{id}");
                directory.other_contacts.push(person(
                    who,
                    &format!("otherContacts/{contact}"),
                    "OTHER_CONTACT",
                    &contact,
                ));
            }
        }
    }
    // Colleagues Maya works with closely are also saved contacts.
    for key in ["daniel", "priya", "tom", "lena"] {
        let who = cast::get(key);
        let id = format!(
            "c{}",
            2_000_000_000_000_000_000u64 + fnv(who.email) % 7_000_000_000_000_000_000
        );
        directory
            .contacts
            .push(person(who, &format!("people/{id}"), "CONTACT", &id));
    }
    directory
        .contacts
        .sort_by(|a, b| a.names[0].display_name.cmp(&b.names[0].display_name));
    directory
}

/// The profile of `email`, another address signed in on the mock's mailbox: just the address,
/// and Google's generated avatar.
pub fn someone_else(email: &str) -> Person {
    let primary = || {
        Some(FieldMetadata {
            primary: Some(true),
            ..FieldMetadata::default()
        })
    };
    Person {
        resource_name: Some(format!(
            "people/{}",
            1_000_000_000_000_000_000u64 + fnv(email) % 8_000_000_000_000_000_000
        )),
        etag: Some(format!("%Eg{:x}", fnv(email))),
        email_addresses: vec![EmailAddress {
            metadata: primary(),
            value: Some(email.into()),
            ..EmailAddress::default()
        }],
        photos: vec![Photo {
            metadata: primary(),
            url: Some(DEFAULT_PHOTO.into()),
            default: Some(true),
        }],
        ..Person::default()
    }
}

fn person(who: &cast::Person, resource: &str, source: &str, id: &str) -> Person {
    let etag = format!("%Eg{:x}", fnv(resource));
    let source = Source {
        kind: Some(source.into()),
        id: Some(id.trim_start_matches('c').into()),
        etag: Some(etag.clone()),
        update_time: Some("2026-10-02T15:04:11.512Z".into()),
    };
    let metadata = || {
        Some(FieldMetadata {
            primary: Some(true),
            source: Some(source.clone()),
            ..FieldMetadata::default()
        })
    };
    Person {
        resource_name: Some(resource.into()),
        etag: Some(etag),
        metadata: Some(PersonMetadata {
            sources: vec![source.clone()],
            object_type: Some("PERSON".into()),
            ..PersonMetadata::default()
        }),
        names: vec![Name {
            metadata: metadata(),
            display_name: Some(who.name()),
            display_name_last_first: Some(format!("{}, {}", who.last, who.first)),
            family_name: Some(who.last.into()),
            given_name: Some(who.first.into()),
            unstructured_name: Some(who.name()),
            ..Name::default()
        }],
        email_addresses: vec![EmailAddress {
            metadata: metadata(),
            value: Some(who.email.into()),
            kind: Some(
                if who.circle == Circle::Personal {
                    "home"
                } else {
                    "work"
                }
                .into(),
            ),
            formatted_type: Some(
                if who.circle == Circle::Personal {
                    "Home"
                } else {
                    "Work"
                }
                .into(),
            ),
            ..EmailAddress::default()
        }],
        phone_numbers: (!who.phone.is_empty())
            .then(|| PhoneNumber {
                metadata: metadata(),
                value: Some(who.phone.into()),
                canonical_form: Some(who.phone.replace([' ', '-'], "")),
                kind: Some("mobile".into()),
                formatted_type: Some("Mobile".into()),
            })
            .into_iter()
            .collect(),
        photos: vec![Photo {
            metadata: metadata(),
            url: Some(DEFAULT_PHOTO.into()),
            default: Some(true),
        }],
        organizations: (!who.org.is_empty())
            .then(|| Organization {
                metadata: metadata(),
                name: Some(who.org.into()),
                title: Some(who.title.into()),
                current: Some(true),
                kind: Some("work".into()),
                domain: who.email.split_once('@').map(|(_, d)| d.to_owned()),
                ..Organization::default()
            })
            .into_iter()
            .collect(),
    }
}
