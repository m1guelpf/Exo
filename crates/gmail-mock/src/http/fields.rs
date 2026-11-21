//! The `fields` partial-response parameter: `id,messages(id,payload/headers),nextPageToken`.

use std::collections::BTreeMap;

use serde_json::Value;

/// A parsed field mask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mask {
    /// Keep the whole value.
    All,
    /// Keep only these keys (`*` matches any), each narrowed by its own mask.
    Some(BTreeMap<String, Mask>),
}

impl Mask {
    /// Parses a mask. Malformed input degrades to keeping what was recognisable.
    pub fn parse(spec: &str) -> Self {
        let mut chars = spec.chars().filter(|ch| !ch.is_whitespace()).peekable();
        parse_list(&mut chars)
    }

    /// Narrows `value` to the mask. Arrays apply the mask to every element.
    pub fn apply(&self, value: Value) -> Value {
        let Self::Some(keys) = self else {
            return value;
        };
        match value {
            Value::Array(items) => {
                Value::Array(items.into_iter().map(|item| self.apply(item)).collect())
            }
            Value::Object(object) => Value::Object(
                object
                    .into_iter()
                    .filter_map(|(key, value)| {
                        let mask = keys.get(&key).or_else(|| keys.get("*"))?;
                        Some((key, mask.apply(value)))
                    })
                    .collect(),
            ),
            scalar => scalar,
        }
    }

    fn insert(&mut self, path: &[String], leaf: Mask) {
        let Self::Some(keys) = self else {
            return; // Already keeping everything.
        };
        let Some((first, rest)) = path.split_first() else {
            return;
        };
        let entry = keys
            .entry(first.clone())
            .or_insert_with(|| Self::Some(BTreeMap::new()));
        if rest.is_empty() {
            *entry = match (std::mem::replace(entry, Self::All), leaf) {
                (Self::Some(mut existing), Self::Some(more)) if !existing.is_empty() => {
                    existing.extend(more);
                    Self::Some(existing)
                }
                (_, leaf) => leaf,
            };
        } else {
            entry.insert(rest, leaf);
        }
    }
}

fn parse_list(chars: &mut std::iter::Peekable<impl Iterator<Item = char>>) -> Mask {
    let mut mask = Mask::Some(BTreeMap::new());
    loop {
        let mut path = vec![String::new()];
        while let Some(&ch) = chars.peek() {
            match ch {
                ',' | '(' | ')' => break,
                '/' => path.push(String::new()),
                _ => path.last_mut().expect("path starts non-empty").push(ch),
            }
            chars.next();
        }
        let leaf = if chars.peek() == Some(&'(') {
            chars.next();
            let inner = parse_list(chars);
            chars.next(); // ')'
            inner
        } else {
            Mask::All
        };
        if path.iter().all(|segment| !segment.is_empty()) {
            mask.insert(&path, leaf);
        }
        match chars.peek() {
            Some(',') => {
                chars.next();
            }
            _ => return mask,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn selects_nested_fields() {
        let value = json!({
            "messages": [
                {"id": "a", "threadId": "t", "payload": {"headers": [1], "body": {}}},
                {"id": "b", "threadId": "t"}
            ],
            "nextPageToken": "n",
            "resultSizeEstimate": 2
        });
        let mask = Mask::parse("messages(id,payload/headers),nextPageToken");
        assert_eq!(
            mask.apply(value),
            json!({
                "messages": [{"id": "a", "payload": {"headers": [1]}}, {"id": "b"}],
                "nextPageToken": "n"
            })
        );
    }

    #[test]
    fn whole_field_wins_over_subselection() {
        let mask = Mask::parse("a/b,a");
        assert_eq!(
            mask.apply(json!({"a": {"b": 1, "c": 2}})),
            json!({"a": {"b": 1, "c": 2}})
        );
    }
}
