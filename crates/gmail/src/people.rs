//! Google People v1: the user's profile, contacts, "other contacts" and the Workspace
//! directory (`discovery/people.v1.json`). Only the person fields Exo reads are modeled.

use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::client::{Call, Client};
use crate::endpoints::join;

/// A person or contact.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    /// `people/{id}` or `otherContacts/{id}`.
    pub resource_name: Option<String>,
    /// ETag.
    pub etag: Option<String>,
    /// Sources and deletion state.
    pub metadata: Option<PersonMetadata>,
    /// Names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<Name>,
    /// Email addresses.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub email_addresses: Vec<EmailAddress>,
    /// Phone numbers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phone_numbers: Vec<PhoneNumber>,
    /// Photos.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub photos: Vec<Photo>,
    /// Organizations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub organizations: Vec<Organization>,
}

/// `Person.metadata`.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonMetadata {
    /// Where the person's data comes from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<Source>,
    /// Former resource names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub previous_resource_names: Vec<String>,
    /// Linked profiles.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linked_people_resource_names: Vec<String>,
    /// Set in sync responses for people deleted since the token.
    pub deleted: Option<bool>,
    /// Deprecated object type (`PERSON`).
    pub object_type: Option<String>,
}

/// A data source.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    /// `ACCOUNT`, `PROFILE`, `DOMAIN_PROFILE`, `CONTACT`, `OTHER_CONTACT`, `DOMAIN_CONTACT`.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Source ID.
    pub id: Option<String>,
    /// ETag.
    pub etag: Option<String>,
    /// Last update, RFC 3339.
    pub update_time: Option<String>,
}

/// Per-field metadata.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldMetadata {
    /// The person's primary value of this field.
    pub primary: Option<bool>,
    /// Primary within its source.
    pub source_primary: Option<bool>,
    /// Verified.
    pub verified: Option<bool>,
    /// Where the value comes from.
    pub source: Option<Source>,
}

/// A name.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Name {
    /// Field metadata.
    pub metadata: Option<FieldMetadata>,
    /// Display name.
    pub display_name: Option<String>,
    /// "Last, First".
    pub display_name_last_first: Option<String>,
    /// Family name.
    pub family_name: Option<String>,
    /// Given name.
    pub given_name: Option<String>,
    /// Middle name.
    pub middle_name: Option<String>,
    /// Name as entered.
    pub unstructured_name: Option<String>,
}

/// An email address.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailAddress {
    /// Field metadata.
    pub metadata: Option<FieldMetadata>,
    /// The address.
    pub value: Option<String>,
    /// `home`, `work`, `other` or custom.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Localized type.
    pub formatted_type: Option<String>,
    /// Display name for this address.
    pub display_name: Option<String>,
}

/// A phone number.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneNumber {
    /// Field metadata.
    pub metadata: Option<FieldMetadata>,
    /// As entered.
    pub value: Option<String>,
    /// E.164 form.
    pub canonical_form: Option<String>,
    /// `mobile`, `work`, ...
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Localized type.
    pub formatted_type: Option<String>,
}

/// A photo.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Photo {
    /// Field metadata.
    pub metadata: Option<FieldMetadata>,
    /// URL; append `=s{size}` to resize.
    pub url: Option<String>,
    /// Whether this is the generated default avatar.
    pub default: Option<bool>,
}

/// An organization.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Organization {
    /// Field metadata.
    pub metadata: Option<FieldMetadata>,
    /// Name.
    pub name: Option<String>,
    /// Job title.
    pub title: Option<String>,
    /// Department.
    pub department: Option<String>,
    /// Current organization.
    pub current: Option<bool>,
    /// `work`, `school` or custom.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Domain.
    pub domain: Option<String>,
}

/// `people.connections.list`.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListConnectionsResponse {
    /// The page of contacts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<Person>,
    /// Token for the next page.
    pub next_page_token: Option<String>,
    /// Token for incremental sync, on the last page when requested.
    pub next_sync_token: Option<String>,
    /// Deprecated total.
    pub total_people: Option<u32>,
    /// Total contacts.
    pub total_items: Option<u32>,
}

/// `otherContacts.list`.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListOtherContactsResponse {
    /// The page of other contacts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub other_contacts: Vec<Person>,
    /// Token for the next page.
    pub next_page_token: Option<String>,
    /// Token for incremental sync, on the last page when requested.
    pub next_sync_token: Option<String>,
    /// Total other contacts.
    pub total_size: Option<u32>,
}

/// `people.listDirectoryPeople`.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDirectoryPeopleResponse {
    /// The page of directory people.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub people: Vec<Person>,
    /// Token for the next page.
    pub next_page_token: Option<String>,
    /// Token for incremental sync, on the last page when requested.
    pub next_sync_token: Option<String>,
}

/// `people.searchContacts` and `otherContacts.search`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResponse {
    /// Matches, best first. Omitted when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub results: Vec<SearchResult>,
}

/// One search match.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    /// The matching person.
    pub person: Person,
}

/// Paging and sync parameters shared by the People list calls.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PeopleQuery {
    /// Page size (max 1000).
    pub page_size: Option<u32>,
    /// Page token.
    pub page_token: Option<String>,
    /// Ask for a `nextSyncToken` on the last page.
    pub request_sync_token: bool,
    /// Return changes since this token. An expired token fails with 400 `EXPIRED_SYNC_TOKEN`.
    pub sync_token: Option<String>,
}

/// People calls. Get one with [`Client::people`].
#[derive(Debug, Clone, Copy)]
pub struct People<'a> {
    client: &'a Client,
}

impl Client {
    /// People calls for the authenticated user.
    #[must_use]
    pub fn people(&self) -> People<'_> {
        People { client: self }
    }
}

impl<'a> People<'a> {
    fn call<T>(&self, path: &str) -> Call<'a, T> {
        // Custom methods (`people:searchContacts`) are a single path segment.
        let url = join(
            &self.client.endpoints.people,
            ["v1"].into_iter().chain(path.split('/')),
        );
        Call::new(self.client, Method::GET, url)
    }

    /// `people.get` for `people/me`. `fields` is the `personFields` mask, e.g. `names,photos`.
    pub fn me(&self, fields: &str) -> Call<'a, Person> {
        self.call("people/me").query("personFields", fields)
    }

    /// `people.connections.list`: the user's contacts.
    pub fn connections(
        &self,
        fields: &str,
        query: &PeopleQuery,
    ) -> Call<'a, ListConnectionsResponse> {
        paged(
            self.call("people/me/connections")
                .query("personFields", fields),
            query,
        )
    }

    /// `otherContacts.list`: people the user has interacted with.
    pub fn other_contacts(
        &self,
        fields: &str,
        query: &PeopleQuery,
    ) -> Call<'a, ListOtherContactsResponse> {
        paged(self.call("otherContacts").query("readMask", fields), query)
    }

    /// `otherContacts.search`.
    pub fn search_other_contacts(&self, query: &str, fields: &str) -> Call<'a, SearchResponse> {
        self.call("otherContacts:search")
            .query("query", query)
            .query("readMask", fields)
    }

    /// `people.searchContacts`.
    pub fn search_contacts(&self, query: &str, fields: &str) -> Call<'a, SearchResponse> {
        self.call("people:searchContacts")
            .query("query", query)
            .query("readMask", fields)
    }

    /// `people.listDirectoryPeople` from the domain profile and contact sources.
    pub fn directory(
        &self,
        fields: &str,
        query: &PeopleQuery,
    ) -> Call<'a, ListDirectoryPeopleResponse> {
        paged(
            self.call("people:listDirectoryPeople")
                .query("readMask", fields)
                .query_all(
                    "sources",
                    [
                        "DIRECTORY_SOURCE_TYPE_DOMAIN_CONTACT",
                        "DIRECTORY_SOURCE_TYPE_DOMAIN_PROFILE",
                    ],
                ),
            query,
        )
    }
}

fn paged<'a, T>(call: Call<'a, T>, query: &PeopleQuery) -> Call<'a, T> {
    call.query_opt("pageSize", query.page_size)
        .query_opt("pageToken", query.page_token.as_ref())
        .query_opt("requestSyncToken", query.request_sync_token.then_some(true))
        .query_opt("syncToken", query.sync_token.as_ref())
}
