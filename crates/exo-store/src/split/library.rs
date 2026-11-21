//! The Split Inbox Library: templates for splits, by category and role (`library.splits`). Exo
//! ships the catalogue as `assets/split-library.json`, or reads another of its shape; this module
//! gives the templates shape and their rules.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use super::SplitInbox;

/// `library.splits`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SplitLibrary {
    /// Every template by id.
    pub templates: BTreeMap<String, Template>,
    /// The sidebar's categories by id.
    pub categories: BTreeMap<String, Section>,
    /// The categories' order.
    pub category_list: Vec<String>,
    /// The sidebar's roles by id.
    pub roles: BTreeMap<String, Section>,
    /// The roles' order.
    pub role_list: Vec<String>,
}

/// A category or role: a named list of templates.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Section {
    /// Its id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// The sidebar's name for it, when it differs.
    pub sidebar_name: Option<String>,
    /// Its templates, in order.
    pub template_ids: Vec<String>,
}

/// A template for a split.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Template {
    /// `splitTemplate_…`.
    pub id: String,
    /// The split's name, and the card's.
    pub name: String,
    /// The card's and the settings' description.
    pub description: String,
    /// The built-in kind it sets up (`vip`, `team`, `news`, `calendar`, `starred`, `shared`,
    /// `importantOther`, `inbox`, `reminders`).
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// What the split matches.
    pub properties: Properties,
    /// The card's preview.
    pub preview: Preview,
}

/// What a template's split matches.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Properties {
    /// Senders, `from:` each.
    pub senders: Option<Senders>,
    /// A query besides the senders, and what the settings say about it.
    pub query: Option<QueryProperty>,
    /// Auto labels the split uses, which Exo hasn't got.
    pub auto_labels: Option<AutoLabels>,
    /// The split's threads also stay in Important and Other, unless the split says otherwise.
    pub show_in_important_other: Option<bool>,
}

/// A template's senders.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Senders {
    /// Addresses and domains.
    pub value: Vec<String>,
    /// The definition field's placeholder.
    pub placeholder: Option<String>,
    /// The user adds and removes senders.
    pub is_editable: bool,
    /// Only domains can be added.
    pub domains_only: bool,
}

/// A template's query.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QueryProperty {
    /// The query.
    pub value: String,
    /// Shown under the definition.
    pub description: Option<String>,
}

/// A template's auto labels.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct AutoLabels {
    /// Their ids.
    pub value: Vec<String>,
}

/// A template card's preview.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Preview {
    /// `#rrggbbaa`: the card's color, and its alpha for the backdrop.
    pub color: String,
    /// Up to three rows; missing ones (or none) show skeleton bars.
    pub threads: Option<Vec<Option<PreviewThread>>>,
}

/// A row of a preview.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PreviewThread {
    /// The sender; a skeleton bar without one.
    pub from: Option<String>,
    /// The subject; a skeleton bar without one.
    pub subject: Option<String>,
    /// A label chip before the subject.
    pub label: Option<String>,
    /// The chip's color.
    pub label_color: Option<String>,
    /// A shared conversation's icon.
    pub is_shared: bool,
    /// An event's icon.
    pub is_event: bool,
    /// A starred dot.
    pub is_starred: bool,
}

/// A category or role with its templates, as the library shows them.
#[derive(Debug, Clone, Copy)]
pub struct LibrarySection<'a> {
    /// The section.
    pub section: &'a Section,
    /// Its templates that the library shows.
    pub templates: &'a [String],
}

impl std::str::FromStr for SplitLibrary {
    type Err = serde_json::Error;

    /// Reads `library.splits` as JSON.
    fn from_str(json: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(json)
    }
}

impl SplitLibrary {
    /// The library as this account sees it: templates that need auto labels are left out, as
    /// Exo has none (News, which works without them, stays), and so are Shared (a team feature)
    /// and Reminders; sections keep the templates that are left. Team's senders are
    /// `team_domains`, the account's.
    #[must_use]
    pub fn for_account(mut self, team_domains: &[String]) -> Self {
        self.templates.retain(|_, template| {
            let auto_labels = template.properties.auto_labels.is_some()
                && template.kind.as_deref() != Some("news");
            !auto_labels && !matches!(template.kind.as_deref(), Some("shared" | "reminders"))
        });
        if let Some(team) = self
            .templates
            .values_mut()
            .find(|template| template.kind.as_deref() == Some("team"))
            && let Some(senders) = &mut team.properties.senders
        {
            senders.value = team_domains.to_vec();
        }
        let templates = &self.templates;
        for section in self.categories.values_mut().chain(self.roles.values_mut()) {
            section.template_ids.retain(|id| templates.contains_key(id));
        }
        self
    }

    fn sections<'a>(
        &'a self,
        by_id: &'a BTreeMap<String, Section>,
        order: &'a [String],
    ) -> Vec<LibrarySection<'a>> {
        order
            .iter()
            .filter_map(|id| by_id.get(id))
            .filter(|section| !section.template_ids.is_empty())
            .map(|section| LibrarySection {
                section,
                templates: &section.template_ids,
            })
            .collect()
    }

    /// The categories with templates, in order.
    pub fn categories(&self) -> Vec<LibrarySection<'_>> {
        self.sections(&self.categories, &self.category_list)
    }

    /// The roles with templates, in order.
    pub fn roles(&self) -> Vec<LibrarySection<'_>> {
        self.sections(&self.roles, &self.role_list)
    }

    /// The templates whose name, description, senders, auto labels or query contain `text`,
    /// ignoring case, each once in the sections' order.
    pub fn search(&self, text: &str) -> Vec<&Template> {
        if text.trim().is_empty() {
            return Vec::new();
        }
        let text = text.to_lowercase();
        let has = |value: &str| value.to_lowercase().contains(&text);
        let mut seen = BTreeSet::new();
        self.categories()
            .into_iter()
            .chain(self.roles())
            .flat_map(|section| section.templates.iter())
            .filter(|id| seen.insert(id.as_str()))
            .filter_map(|id| self.templates.get(id))
            .filter(|template| {
                let properties = &template.properties;
                has(&template.name)
                    || has(&template.description)
                    || properties
                        .senders
                        .as_ref()
                        .is_some_and(|senders| senders.value.iter().any(|sender| has(sender)))
                    || properties
                        .auto_labels
                        .as_ref()
                        .is_some_and(|labels| labels.value.iter().any(|label| has(label)))
                    || properties
                        .query
                        .as_ref()
                        .is_some_and(|query| has(&query.value))
            })
            .collect()
    }

    /// The template that describes `split`: by kind, else by template.
    pub fn template_for(&self, split: &SplitInbox) -> Option<&Template> {
        self.templates.values().find(|template| {
            (template.kind.is_some() && template.kind == split.kind)
                || split.template_id.as_deref() == Some(template.id.as_str())
        })
    }

    /// The template `id` names, or, for a built-in kind, the template of that kind.
    pub fn template(&self, id: &str) -> Option<&Template> {
        self.templates.get(id).or_else(|| {
            self.templates
                .values()
                .find(|template| template.kind.as_deref() == Some(id))
        })
    }
}

impl Template {
    /// The split this template set up, among `splits`: the one of its kind, or from it.
    pub fn split_in<'a>(
        &self,
        splits: impl IntoIterator<Item = &'a SplitInbox>,
    ) -> Option<&'a SplitInbox> {
        splits
            .into_iter()
            .find(|split| match (&self.kind, &split.kind) {
                (Some(kind), Some(split_kind)) => kind == split_kind,
                _ => split.template_id.as_deref() == Some(self.id.as_str()),
            })
    }

    /// The definition for `senders` (without auto labels): `from:` each sender, then the
    /// template's query.
    pub fn query_with(&self, senders: &[String]) -> String {
        let mut unique: Vec<&str> = Vec::new();
        for sender in senders {
            if !unique.contains(&sender.trim()) {
                unique.push(sender.trim());
            }
        }
        let senders = unique
            .iter()
            .map(|sender| format!("from:{sender}"))
            .collect::<Vec<_>>()
            .join(" OR ");
        let query = self
            .properties
            .query
            .as_ref()
            .map_or("", |query| query.value.as_str());
        match (senders.is_empty(), query.is_empty()) {
            (false, false) => format!("{senders} {query}"),
            (false, true) => senders,
            (true, _) => query.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::split::{Definitions, SplitInbox};

    /// A made-up library in the catalogue's shape.
    fn library() -> SplitLibrary {
        serde_json::from_value(serde_json::json!({
            "templates": {
                "splitTemplate_team": { "id": "splitTemplate_team", "name": "Team", "description": "Sample", "type": "team", "properties": { "senders": { "isEditable": true, "domainsOnly": true, "value": ["sample.invalid"] }, "query": { "value": "to:me" } }, "preview": { "color": "#73B6DB4D", "threads": null } },
                "splitTemplate_tool": { "id": "splitTemplate_tool", "name": "Tool", "description": "Sample tool mail", "properties": { "senders": { "value": ["bot@tool.example"] } }, "preview": { "color": "#E9E9E9FF" } },
                "splitTemplate_ai": { "id": "splitTemplate_ai", "name": "Sorted", "description": "Sample", "properties": { "autoLabels": { "value": ["autoLabel_x"] } }, "preview": { "color": "#E9E9E9FF" } },
                "splitTemplate_news": { "id": "splitTemplate_news", "name": "News", "description": "Sample", "type": "news", "properties": { "autoLabels": { "value": ["autoLabel_y"] }, "senders": { "isEditable": true } }, "preview": { "color": "#E4C45038" } },
                "splitTemplate_shared": { "id": "splitTemplate_shared", "name": "Shared", "description": "Sample", "type": "shared", "properties": { "query": { "value": "is:shared" } }, "preview": { "color": "#8DA2D84D" } }
            },
            "categories": {
                "a": { "id": "a", "name": "A", "templateIds": ["splitTemplate_team", "splitTemplate_ai", "splitTemplate_news", "splitTemplate_shared"] },
                "b": { "id": "b", "name": "B", "templateIds": ["splitTemplate_ai"] }
            },
            "categoryList": ["a", "b"],
            "roles": { "r": { "id": "r", "name": "R", "templateIds": ["splitTemplate_tool", "splitTemplate_team"] } },
            "roleList": ["r"]
        }))
        .expect("parses")
    }

    #[test]
    fn the_library_leaves_out_what_the_account_cant_use() {
        let library = library().for_account(&["brightmoor.co".to_owned()]);
        let names = |sections: Vec<LibrarySection<'_>>| {
            sections
                .iter()
                .map(|section| (section.section.name.clone(), section.templates.to_vec()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(library.categories()),
            [(
                "A".to_owned(),
                vec![
                    "splitTemplate_team".to_owned(),
                    "splitTemplate_news".to_owned()
                ]
            )],
            "auto labels (but News) and Shared go, and an empty section with them"
        );
        assert_eq!(library.roles().len(), 1);
        let team = library.template("team").expect("by kind");
        assert_eq!(
            team.properties
                .senders
                .as_ref()
                .map(|senders| senders.value.clone()),
            Some(vec!["brightmoor.co".to_owned()])
        );
        assert_eq!(
            team.query_with(&[
                "brightmoor.co".to_owned(),
                "brightmoor.co".to_owned(),
                "sample.co".to_owned()
            ]),
            "from:brightmoor.co OR from:sample.co to:me"
        );
        assert_eq!(
            library
                .template("splitTemplate_tool")
                .expect("by id")
                .query_with(&["bot@tool.example".to_owned()]),
            "from:bot@tool.example"
        );
    }

    #[test]
    fn search_matches_names_descriptions_and_senders_once_each() {
        let library = library().for_account(&[]);
        let ids = |text: &str| {
            library
                .search(text)
                .iter()
                .map(|template| template.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids("TOOL"),
            ["splitTemplate_tool"],
            "name and description, ignoring case, listed once"
        );
        assert_eq!(ids("bot@"), ["splitTemplate_tool"], "senders");
        assert_eq!(ids("to:me"), ["splitTemplate_team"], "the query");
        assert!(ids(" ").is_empty());
    }

    #[test]
    fn a_split_finds_its_template_by_kind_or_template() {
        let library = library().for_account(&[]);
        let definitions = Definitions {
            labels: &[],
            me: &[],
        };
        let team = SplitInbox::new("Team", "", Some("team"), None, definitions);
        let tool = SplitInbox::new(
            "Tool",
            "from:bot@tool.example",
            None,
            Some("splitTemplate_tool"),
            definitions,
        );
        let own = SplitInbox::new("Own", "from:own.example", None, None, definitions);
        assert_eq!(
            library
                .template_for(&team)
                .map(|template| template.id.as_str()),
            Some("splitTemplate_team")
        );
        assert_eq!(
            library
                .template_for(&tool)
                .map(|template| template.id.as_str()),
            Some("splitTemplate_tool")
        );
        assert!(library.template_for(&own).is_none());
        let splits = [own.clone(), tool.clone()];
        assert_eq!(
            library.templates["splitTemplate_tool"]
                .split_in(&splits)
                .map(SplitInbox::name),
            Some("Tool")
        );
    }
}
