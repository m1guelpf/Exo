//! Shared test helpers: a running mock, raw HTTP access, and a validator that checks JSON
//! against Google's discovery documents, independent of the `gmail` crate's types.

#![allow(dead_code)] // Each test binary uses a different subset of these helpers.

use std::path::PathBuf;

use gmail_mock::{Config, DEV_ACCESS_TOKEN, MockServer};
use serde_json::Value;

pub async fn server() -> MockServer {
    MockServer::start(Config::default())
        .await
        .expect("mock starts")
}

pub fn client(server: &MockServer) -> gmail::Client {
    gmail::Client::new(
        server.endpoints(),
        gmail::StaticToken(DEV_ACCESS_TOKEN.into()),
    )
}

/// Raw requests, so wire checks don't go through the crate's own serde types.
pub struct Raw {
    pub http: reqwest::Client,
    pub base: url::Url,
}

impl Raw {
    pub fn new(server: &MockServer) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: server.url(),
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base.as_str().trim_end_matches('/'), path)
    }

    pub async fn get(&self, path: &str) -> (u16, Value) {
        let response = self
            .http
            .get(self.url(path))
            .bearer_auth(DEV_ACCESS_TOKEN)
            .send()
            .await
            .expect("request");
        let status = response.status().as_u16();
        (status, response.json().await.unwrap_or(Value::Null))
    }

    pub async fn send(&self, method: reqwest::Method, path: &str, body: Value) -> (u16, Value) {
        let response = self
            .http
            .request(method, self.url(path))
            .bearer_auth(DEV_ACCESS_TOKEN)
            .json(&body)
            .send()
            .await
            .expect("request");
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }
}

/// A discovery document, used as the schema of record.
pub struct Discovery {
    document: Value,
    /// Gmail and People drop empty lists; Calendar sends `"items": []`.
    omits_empty_lists: bool,
}

impl Discovery {
    pub fn gmail() -> Self {
        Self::load("gmail.v1.json", true)
    }

    pub fn calendar() -> Self {
        Self::load("calendar.v3.json", false)
    }

    pub fn people() -> Self {
        Self::load("people.v1.json", true)
    }

    fn load(file: &str, omits_empty_lists: bool) -> Self {
        let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "gmail", "discovery", file]
            .iter()
            .collect();
        let text = std::fs::read_to_string(&path).expect("discovery document present");
        Self {
            document: serde_json::from_str(&text).expect("discovery document is JSON"),
            omits_empty_lists,
        }
    }

    /// Panics with every violation of `schema` in `value`: unknown or misspelled fields, nulls,
    /// wrong JSON types (int64 must be a string, int32 a number), bad enums, non-base64 bytes.
    pub fn assert(&self, schema: &str, value: &Value) {
        let mut errors = Vec::new();
        self.check_ref(schema, value, schema, &mut errors);
        assert!(
            errors.is_empty(),
            "{schema} violations:\n{}\nin {value:#}",
            errors.join("\n")
        );
    }

    fn check_ref(&self, schema: &str, value: &Value, path: &str, errors: &mut Vec<String>) {
        let Some(definition) = self.document["schemas"].get(schema) else {
            errors.push(format!("{path}: unknown schema {schema}"));
            return;
        };
        self.check(definition, value, path, errors);
    }

    fn check(&self, spec: &Value, value: &Value, path: &str, errors: &mut Vec<String>) {
        if let Some(reference) = spec["$ref"].as_str() {
            return self.check_ref(reference, value, path, errors);
        }
        if value.is_null() {
            errors.push(format!("{path}: null (Google omits unset fields)"));
            return;
        }
        if let Some(allowed) = spec["enum"].as_array()
            && !allowed.contains(value)
        {
            errors.push(format!("{path}: {value} not in enum"));
        }
        let kind = spec["type"].as_str().unwrap_or("any");
        match kind {
            "string" => {
                let Some(text) = value.as_str() else {
                    errors.push(format!("{path}: expected string, got {value}"));
                    return;
                };
                match spec["format"].as_str() {
                    Some("int64" | "uint64") if text.parse::<i128>().is_err() => {
                        errors.push(format!("{path}: {text:?} is not an int64 string"));
                    }
                    Some("byte") if gmail::wire::decode_base64url(text).is_err() => {
                        errors.push(format!("{path}: not base64"));
                    }
                    _ => {}
                }
            }
            "integer" | "number" => {
                if !value.is_number() {
                    errors.push(format!("{path}: expected number, got {value}"));
                }
            }
            "boolean" => {
                if !value.is_boolean() {
                    errors.push(format!("{path}: expected boolean, got {value}"));
                }
            }
            "array" => match value.as_array() {
                Some(items) => {
                    if items.is_empty() && self.omits_empty_lists {
                        errors.push(format!("{path}: empty array (Google omits empty lists)"));
                    }
                    for (index, item) in items.iter().enumerate() {
                        self.check(&spec["items"], item, &format!("{path}[{index}]"), errors);
                    }
                }
                None => errors.push(format!("{path}: expected array, got {value}")),
            },
            "object" => {
                let Some(object) = value.as_object() else {
                    errors.push(format!("{path}: expected object, got {value}"));
                    return;
                };
                for (key, item) in object {
                    let child = format!("{path}.{key}");
                    if let Some(property) = spec["properties"].get(key) {
                        self.check(property, item, &child, errors);
                    } else if !spec["additionalProperties"].is_null() {
                        self.check(&spec["additionalProperties"], item, &child, errors);
                    } else {
                        errors.push(format!("{child}: not in the schema"));
                    }
                }
            }
            _ => {}
        }
    }
}
