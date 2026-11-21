//! A small toolkit over html5ever's reference tree, which the render pipeline edits as a script
//! would edit the DOM.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use html5ever::{
    Attribute, LocalName, QualName, ns, parse_document,
    serialize::{SerializeOpts, TraversalScope, serialize},
    tendril::TendrilSink as _,
};
use markup5ever_rcdom::{Handle, Node, NodeData, RcDom, SerializableHandle};

/// A node in the tree.
pub(crate) type NodeRef = Handle;

/// Identifies a node while it's alive, for maps keyed by node.
pub(crate) type NodeKey = *const Node;

/// The node's key.
pub(crate) fn key(node: &NodeRef) -> NodeKey {
    Rc::as_ptr(node)
}

/// Parses a whole document (`<html>`, `<head>`, `<body>` are always there afterwards).
pub(crate) fn parse(html: &str) -> NodeRef {
    parse_document(RcDom::default(), Default::default())
        .one(html)
        .document
}

/// The node's outer HTML (or, for the document, its children's).
pub(crate) fn outer_html(node: &NodeRef) -> String {
    let scope = if matches!(node.data, NodeData::Document) {
        TraversalScope::ChildrenOnly(None)
    } else {
        TraversalScope::IncludeNode
    };
    write(node, scope)
}

/// The node's children as HTML.
pub(crate) fn inner_html(node: &NodeRef) -> String {
    write(node, TraversalScope::ChildrenOnly(None))
}

fn write(node: &NodeRef, traversal_scope: TraversalScope) -> String {
    let mut out = Vec::new();
    let options = SerializeOpts {
        traversal_scope,
        ..SerializeOpts::default()
    };
    serialize(&mut out, &SerializableHandle::from(node.clone()), options)
        .expect("writing to a Vec doesn't fail");
    String::from_utf8(out).expect("the serializer writes UTF-8")
}

/// Escapes text for a double-quoted attribute value.
pub(crate) fn escape_attribute(text: &str) -> String {
    text.replace('&', "&amp;").replace('"', "&quot;")
}

/// A new element in the HTML namespace.
pub(crate) fn element(name: &str, attributes: &[(&str, &str)]) -> NodeRef {
    Node::new(NodeData::Element {
        name: html_name(name),
        attrs: RefCell::new(
            attributes
                .iter()
                .map(|(name, value)| Attribute {
                    name: QualName::new(None, ns!(), LocalName::from(*name)),
                    value: (*value).into(),
                })
                .collect(),
        ),
        template_contents: RefCell::new(None),
        mathml_annotation_xml_integration_point: false,
    })
}

/// A new text node.
pub(crate) fn text_node(text: &str) -> NodeRef {
    Node::new(NodeData::Text {
        contents: RefCell::new(text.into()),
    })
}

fn html_name(name: &str) -> QualName {
    QualName::new(None, ns!(html), LocalName::from(name))
}

/// The element's tag name (lowercase for HTML), or `None` for other nodes.
pub(crate) fn tag(node: &NodeRef) -> Option<&str> {
    match &node.data {
        NodeData::Element { name, .. } => Some(&name.local),
        _ => None,
    }
}

/// Whether `node` is a `name` element.
pub(crate) fn is(node: &NodeRef, name: &str) -> bool {
    tag(node) == Some(name)
}

/// Whether `node` is an element.
pub(crate) fn is_element(node: &NodeRef) -> bool {
    matches!(node.data, NodeData::Element { .. })
}

/// Whether `node` is text.
pub(crate) fn is_text(node: &NodeRef) -> bool {
    matches!(node.data, NodeData::Text { .. })
}

/// Whether `node` is a comment.
pub(crate) fn is_comment(node: &NodeRef) -> bool {
    matches!(node.data, NodeData::Comment { .. })
}

/// The namespaces HTML parsing puts elements in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Namespace {
    Html,
    Svg,
    MathMl,
}

/// The element's namespace (`None` for other nodes).
pub(crate) fn namespace(node: &NodeRef) -> Option<Namespace> {
    match &node.data {
        NodeData::Element { name, .. } if name.ns == ns!(html) => Some(Namespace::Html),
        NodeData::Element { name, .. } if name.ns == ns!(svg) => Some(Namespace::Svg),
        NodeData::Element { name, .. } if name.ns == ns!(mathml) => Some(Namespace::MathMl),
        _ => None,
    }
}

/// The text of a text node.
pub(crate) fn text(node: &NodeRef) -> Option<String> {
    match &node.data {
        NodeData::Text { contents } => Some(contents.borrow().to_string()),
        _ => None,
    }
}

/// Replaces a text node's text.
pub(crate) fn set_text(node: &NodeRef, value: &str) {
    if let NodeData::Text { contents } = &node.data {
        *contents.borrow_mut() = value.into();
    }
}

/// An attribute's value.
pub(crate) fn attr(node: &NodeRef, name: &str) -> Option<String> {
    match &node.data {
        NodeData::Element { attrs, .. } => attrs
            .borrow()
            .iter()
            .find(|attribute| &*attribute.name.local == name)
            .map(|attribute| attribute.value.to_string()),
        _ => None,
    }
}

/// The element's attributes, names and values, in order.
pub(crate) fn attrs(node: &NodeRef) -> Vec<(String, String)> {
    match &node.data {
        NodeData::Element { attrs, .. } => attrs
            .borrow()
            .iter()
            .map(|attribute| {
                (
                    attribute.name.local.to_string(),
                    attribute.value.to_string(),
                )
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Whether the element has the attribute.
pub(crate) fn has_attr(node: &NodeRef, name: &str) -> bool {
    attr(node, name).is_some()
}

/// Sets an attribute.
pub(crate) fn set_attr(node: &NodeRef, name: &str, value: &str) {
    if let NodeData::Element { attrs, .. } = &node.data {
        let mut attrs = attrs.borrow_mut();
        match attrs
            .iter_mut()
            .find(|attribute| &*attribute.name.local == name)
        {
            Some(attribute) => attribute.value = value.into(),
            None => attrs.push(Attribute {
                name: QualName::new(None, ns!(), LocalName::from(name)),
                value: value.into(),
            }),
        }
    }
}

/// Removes an attribute.
pub(crate) fn remove_attr(node: &NodeRef, name: &str) {
    if let NodeData::Element { attrs, .. } = &node.data {
        attrs
            .borrow_mut()
            .retain(|attribute| &*attribute.name.local != name);
    }
}

/// Keeps the attributes `keep` gives a value for (given each name and value), with that value.
pub(crate) fn retain_attrs(node: &NodeRef, mut keep: impl FnMut(&str, &str) -> Option<String>) {
    if let NodeData::Element { attrs, .. } = &node.data {
        attrs.borrow_mut().retain_mut(|attribute| {
            match keep(&attribute.name.local, &attribute.value) {
                Some(value) => {
                    if *attribute.value != *value {
                        attribute.value = value.into();
                    }
                    true
                }
                None => false,
            }
        });
    }
}

/// Whether the element's `class` contains `class`.
pub(crate) fn has_class(node: &NodeRef, class: &str) -> bool {
    attr(node, "class").is_some_and(|classes| classes.split_ascii_whitespace().any(|c| c == class))
}

/// Adds classes (in order, skipping ones already there).
pub(crate) fn add_class(node: &NodeRef, classes: &[&str]) {
    let mut list: Vec<String> = attr(node, "class")
        .map(|value| value.split_ascii_whitespace().map(str::to_owned).collect())
        .unwrap_or_default();
    for class in classes {
        if !list.iter().any(|c| c == class) {
            list.push((*class).to_owned());
        }
    }
    set_attr(node, "class", &list.join(" "));
}

/// Removes classes.
pub(crate) fn remove_class(node: &NodeRef, classes: &[&str]) {
    if let Some(value) = attr(node, "class")
        && value
            .split_ascii_whitespace()
            .any(|class| classes.contains(&class))
    {
        let list: Vec<&str> = value
            .split_ascii_whitespace()
            .filter(|class| !classes.contains(class))
            .collect();
        set_attr(node, "class", &list.join(" "));
    }
}

/// The node's parent.
pub(crate) fn parent(node: &NodeRef) -> Option<NodeRef> {
    let weak = node.parent.take();
    let parent = weak.as_ref().and_then(|weak| weak.upgrade());
    node.parent.set(weak);
    parent
}

/// The node's children.
pub(crate) fn children(node: &NodeRef) -> Vec<NodeRef> {
    node.children.borrow().clone()
}

/// The node's element children.
pub(crate) fn element_children(node: &NodeRef) -> Vec<NodeRef> {
    node.children
        .borrow()
        .iter()
        .filter(|c| is_element(c))
        .cloned()
        .collect()
}

/// Takes the node's children out, for [`set_children`] to put back edited. Their parent links
/// stay until then.
pub(crate) fn take_children(node: &NodeRef) -> Vec<NodeRef> {
    std::mem::take(&mut *node.children.borrow_mut())
}

/// Makes `children` the node's children.
///
/// Editing the child list in one go keeps passes over long sibling lists linear: the tree has no
/// sibling links, so each single-node edit has to find the node among its siblings first.
pub(crate) fn set_children(node: &NodeRef, children: Vec<NodeRef>) {
    for child in &children {
        child.parent.set(Some(Rc::downgrade(node)));
    }
    *node.children.borrow_mut() = children;
}

fn index_in_parent(node: &NodeRef) -> Option<(NodeRef, usize)> {
    let parent = parent(node)?;
    let index = parent
        .children
        .borrow()
        .iter()
        .position(|c| Rc::ptr_eq(c, node))?;
    Some((parent, index))
}

/// The siblings after the node, in order.
pub(crate) fn following_siblings(node: &NodeRef) -> Vec<NodeRef> {
    index_in_parent(node)
        .map(|(parent, index)| parent.children.borrow()[index + 1..].to_vec())
        .unwrap_or_default()
}

/// The siblings before the node, in order.
pub(crate) fn preceding_siblings(node: &NodeRef) -> Vec<NodeRef> {
    index_in_parent(node)
        .map(|(parent, index)| parent.children.borrow()[..index].to_vec())
        .unwrap_or_default()
}

/// The previous element sibling.
pub(crate) fn previous_element_sibling(node: &NodeRef) -> Option<NodeRef> {
    preceding_siblings(node).into_iter().rev().find(is_element)
}

/// Takes the node out of the tree.
pub(crate) fn detach(node: &NodeRef) {
    if let Some((parent, index)) = index_in_parent(node) {
        parent.children.borrow_mut().remove(index);
    }
    node.parent.set(None);
}

/// Takes the nodes out of the tree, visiting each parent's child list once.
pub(crate) fn detach_all(nodes: &[NodeRef]) {
    let gone: HashSet<NodeKey> = nodes.iter().map(key).collect();
    for parent in parents_of(nodes) {
        parent
            .children
            .borrow_mut()
            .retain(|child| !gone.contains(&key(child)));
    }
    for node in nodes {
        node.parent.set(None);
    }
}

/// The nodes' parents, each once.
pub(crate) fn parents_of(nodes: &[NodeRef]) -> Vec<NodeRef> {
    let mut seen = HashSet::new();
    nodes
        .iter()
        .filter_map(parent)
        .filter(|parent| seen.insert(key(parent)))
        .collect()
}

/// Appends `child` to `parent`.
pub(crate) fn append(parent: &NodeRef, child: &NodeRef) {
    detach(child);
    child.parent.set(Some(Rc::downgrade(parent)));
    parent.children.borrow_mut().push(child.clone());
}

/// Inserts `node` before `reference`.
pub(crate) fn insert_before(reference: &NodeRef, node: &NodeRef) {
    insert_at(reference, 0, node);
}

/// Inserts `node` after `reference`.
pub(crate) fn insert_after(reference: &NodeRef, node: &NodeRef) {
    insert_at(reference, 1, node);
}

fn insert_at(reference: &NodeRef, offset: usize, node: &NodeRef) {
    detach(node);
    if let Some((parent, index)) = index_in_parent(reference) {
        node.parent.set(Some(Rc::downgrade(&parent)));
        parent
            .children
            .borrow_mut()
            .insert(index + offset, node.clone());
    }
}

/// Puts each node's replacements (new nodes, not ones in the tree) where it is, editing each
/// parent's child list once.
pub(crate) fn replace_all(mut replacements: Vec<(NodeRef, Vec<NodeRef>)>) {
    let nodes: Vec<NodeRef> = replacements.iter().map(|(node, _)| node.clone()).collect();
    let index: HashMap<NodeKey, usize> = replacements
        .iter()
        .enumerate()
        .map(|(i, (node, _))| (key(node), i))
        .collect();
    for parent in parents_of(&nodes) {
        let mut children = Vec::new();
        for child in take_children(&parent) {
            match index.get(&key(&child)) {
                Some(&i) => children.append(&mut replacements[i].1),
                None => children.push(child),
            }
        }
        set_children(&parent, children);
    }
    for node in &nodes {
        node.parent.set(None);
    }
}

/// Merges adjacent text nodes and drops empty ones, throughout the subtree (`Node.normalize`).
pub(crate) fn normalize(node: &NodeRef) {
    let mut kept: Vec<NodeRef> = Vec::new();
    for child in take_children(node) {
        match text(&child) {
            Some(contents) if contents.is_empty() => {}
            Some(contents) => match kept.last().filter(|last| is_text(last)) {
                Some(previous) => {
                    let merged = text(previous).unwrap_or_default() + &contents;
                    set_text(previous, &merged);
                }
                None => kept.push(child),
            },
            None => {
                normalize(&child);
                kept.push(child);
            }
        }
    }
    set_children(node, kept);
}

/// Replaces each of `nodes` with its children, editing each affected child list once, and
/// returns the parents those children ended up in.
pub(crate) fn unwrap_all(nodes: &[NodeRef]) -> Vec<NodeRef> {
    let gone: HashSet<NodeKey> = nodes.iter().map(key).collect();
    fn expand(node: NodeRef, gone: &HashSet<NodeKey>, into: &mut Vec<NodeRef>) {
        if gone.contains(&key(&node)) {
            for child in take_children(&node) {
                expand(child, gone, into);
            }
        } else {
            into.push(node);
        }
    }
    let mut parents = parents_of(nodes);
    parents.retain(|parent| !gone.contains(&key(parent)));
    for parent in &parents {
        let mut children = Vec::new();
        for child in take_children(parent) {
            expand(child, &gone, &mut children);
        }
        set_children(parent, children);
    }
    parents
}

/// Every node under `node`, in document order (not `node` itself).
pub(crate) fn descendants(node: &NodeRef) -> impl Iterator<Item = NodeRef> {
    let mut stack: Vec<NodeRef> = children(node).into_iter().rev().collect();
    std::iter::from_fn(move || {
        let next = stack.pop()?;
        stack.extend(children(&next).into_iter().rev());
        Some(next)
    })
}

/// The elements under `node` that `matches` accepts, in document order (`querySelectorAll`).
pub(crate) fn select(node: &NodeRef, matches: impl Fn(&NodeRef) -> bool) -> Vec<NodeRef> {
    descendants(node)
        .filter(|n| is_element(n) && matches(n))
        .collect()
}

/// The first descendant element `matches` accepts.
pub(crate) fn select_first(node: &NodeRef, matches: impl Fn(&NodeRef) -> bool) -> Option<NodeRef> {
    descendants(node).find(|n| is_element(n) && matches(n))
}

/// The nearest inclusive ancestor element `matches` accepts (`closest`).
pub(crate) fn closest(node: &NodeRef, matches: impl Fn(&NodeRef) -> bool) -> Option<NodeRef> {
    let mut current = Some(node.clone());
    while let Some(candidate) = current {
        if is_element(&candidate) && matches(&candidate) {
            return Some(candidate);
        }
        current = parent(&candidate);
    }
    None
}

/// The concatenated text of every text node under `node` (`textContent`).
pub(crate) fn text_content(node: &NodeRef) -> String {
    if let Some(text) = text(node) {
        return text;
    }
    descendants(node).filter_map(|n| text(&n)).collect()
}

/// The text of `html` read as a document (the parsed document's text content): its entities
/// decoded, its tags dropped.
pub fn html_text(html: &str) -> String {
    text_content(&parse(html))
}

/// The document's `<body>` (or `<html>` when a fragment has none).
pub(crate) fn body(document: &NodeRef) -> Option<NodeRef> {
    let html = element_children(document)
        .into_iter()
        .find(|n| is(n, "html"))?;
    element_children(&html)
        .into_iter()
        .find(|n| is(n, "body"))
        .or(Some(html))
}

/// The document's `<head>`.
pub(crate) fn head(document: &NodeRef) -> Option<NodeRef> {
    let html = element_children(document)
        .into_iter()
        .find(|n| is(n, "html"))?;
    element_children(&html).into_iter().find(|n| is(n, "head"))
}

/// The declarations of an inline `style` attribute, as `CSSStyleDeclaration` exposes them.
#[derive(Clone, Debug, Default)]
pub(crate) struct Style {
    declarations: Vec<(String, String, bool)>,
}

impl Style {
    /// The element's inline style.
    pub(crate) fn of(node: &NodeRef) -> Self {
        attr(node, "style")
            .map(|css| Self::parse(&css))
            .unwrap_or_default()
    }

    /// Parses a declaration list.
    pub(crate) fn parse(css: &str) -> Self {
        let mut declarations: Vec<(String, String, bool)> = Vec::new();
        for declaration in split_top_level(css, ';') {
            let Some((name, value)) = declaration.split_once(':') else {
                continue;
            };
            let name = name.trim().to_ascii_lowercase();
            let mut value = value.trim().to_owned();
            let important = value.to_ascii_lowercase().ends_with("!important");
            if important {
                value.truncate(value.len() - "!important".len());
                value = value.trim_end().to_owned();
            }
            if name.is_empty() || value.is_empty() {
                continue;
            }
            declarations.push((name, value, important));
        }
        // A property declared again keeps only its last declaration, in that one's place.
        let mut seen = HashSet::new();
        declarations.reverse();
        declarations.retain(|(name, _, _)| seen.insert(name.clone()));
        declarations.reverse();
        Self { declarations }
    }

    /// A property's value (`""` when unset, as the DOM returns it). The `border` shorthand
    /// only reads back when it was set as a shorthand.
    pub(crate) fn get(&self, name: &str) -> &str {
        self.declarations
            .iter()
            .find(|(n, _, _)| n == name)
            .map_or("", |(_, value, _)| value.as_str())
    }

    /// Whether a property is set with `!important`.
    pub(crate) fn is_important(&self, name: &str) -> bool {
        self.declarations
            .iter()
            .any(|(n, _, important)| n == name && *important)
    }

    /// Sets a property.
    pub(crate) fn set(&mut self, name: &str, value: &str, important: bool) {
        self.declarations.retain(|(n, _, _)| n != name);
        self.declarations
            .push((name.to_owned(), value.to_owned(), important));
    }

    /// Removes a property.
    pub(crate) fn remove(&mut self, name: &str) {
        self.declarations.retain(|(n, _, _)| n != name);
    }

    /// Whether no property is set.
    pub(crate) fn is_empty(&self) -> bool {
        self.declarations.is_empty()
    }

    /// The declarations, serialized the way the DOM does.
    pub(crate) fn css(&self) -> String {
        let css: Vec<String> = self
            .declarations
            .iter()
            .map(|(name, value, important)| {
                format!(
                    "{name}: {value}{};",
                    if *important { " !important" } else { "" }
                )
            })
            .collect();
        css.join(" ")
    }

    /// Writes the declarations back to `node`'s `style`.
    pub(crate) fn apply(&self, node: &NodeRef) {
        set_attr(node, "style", &self.css());
    }
}

/// Splits on `separator` outside parentheses and quotes.
fn split_top_level(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut quote, mut start) = (0usize, None, 0);
    for (i, c) in text.char_indices() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth = depth.saturating_sub(1),
            (None, _) if c == separator && depth == 0 => {
                parts.push(&text[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_serializes() {
        let document = parse("<p class=a>Hi <b>there</b></p>");
        let body = body(&document).unwrap();
        assert_eq!(inner_html(&body), "<p class=\"a\">Hi <b>there</b></p>");
        let p = element_children(&body).remove(0);
        add_class(&p, &["b", "a"]);
        assert_eq!(attr(&p, "class").unwrap(), "a b");
        unwrap_all(&[p]);
        assert_eq!(inner_html(&body), "Hi <b>there</b>");
    }

    #[test]
    fn reads_a_documents_text() {
        assert_eq!(
            html_text("Tom &amp; Ann&#39;s <b>call</b> &lt;3"),
            "Tom & Ann's call <3"
        );
    }

    #[test]
    fn reads_inline_styles() {
        let style = Style::parse("color: RED; background: url(a;b.png) ; COLOR:blue !important");
        assert_eq!(style.get("color"), "blue");
        assert!(style.is_important("color"));
        assert_eq!(style.get("background"), "url(a;b.png)");
        assert_eq!(style.get("border"), "");
    }
}
