//! What pasted HTML goes through before the editor's sanitizer: the editor's cleanup of images
//! Word saved to disk, and the paste's pre-processing and colour rules.

use crate::{
    colors,
    dom::{self, NodeRef, Style},
};

/// Prepares a paste's HTML (a whole clipboard document) and returns its body's content: Word's
/// lists rebuilt, Excel's sheet inlined, Google Docs' wrapper gone, blank lines kept between
/// spaced paragraphs, hidden content dropped, and colours that wouldn't read snapped.
pub fn prepare_paste(html: &str) -> String {
    let document = dom::parse(html);
    let Some(body) = dom::body(&document) else {
        return String::new();
    };
    let root = dom::parent(&body).unwrap_or_else(|| body.clone());
    let from = |product: &str, namespaces: [&str; 3]| {
        namespaces.iter().any(|name| {
            dom::attr(&root, name)
                .is_some_and(|value| value.contains(product) && value.contains("microsoft"))
        })
    };
    let files = dom::select(&body, |node| {
        dom::is(node, "img") && dom::attr(node, "src").is_some_and(|src| src.starts_with("file:"))
    });
    if !files.is_empty() {
        // Word's saved images, its VML copies of them and its `OLE_LINK` bookmarks.
        dom::detach_all(&files);
        dom::detach_all(&dom::select(&body, |node| dom::is(node, "v:imagedata")));
        dom::unwrap_all(&dom::select(&body, |node| {
            dom::is(node, "a")
                && dom::attr(node, "name").is_some_and(|name| {
                    name.strip_prefix("OLE_LINK")
                        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                })
        }));
    }
    if from("excel", ["xmlns:o", "xmlns:v", "xmlns:x"]) {
        inline_sheet(&document, &body);
    }
    if from("word", ["xmlns:o", "xmlns:w", "xmlns:m"]) {
        word_lists(&body);
    }
    dom::unwrap_all(&dom::select(&body, |node| {
        dom::is(node, "b")
            && dom::attr(node, "id").is_some_and(|id| id.starts_with("docs-internal-guid"))
    }));
    for div in dom::select(&body, |node| {
        dom::is(node, "div") && Style::of(node).get("display") == "inline"
    }) {
        rename(&div, "span");
    }
    blank_lines(
        &body,
        dom::select_first(&document, |node| dom::is(node, "style")).is_none(),
    );
    for hidden in dom::select(&body, |node| {
        dom::attr(node, "style").is_some_and(|style| style.contains("display: none"))
    }) {
        let parent = dom::parent(&hidden);
        dom::detach(&hidden);
        if let Some(parent) = parent.filter(|parent| dom::children(parent).is_empty()) {
            dom::detach(&parent);
        }
    }
    // An item that starts with a list gives its content to its list.
    for item in dom::select(&body, |node| {
        dom::is(node, "li")
            && dom::children(node)
                .first()
                .is_some_and(|first| dom::is(first, "ul"))
    }) {
        for child in dom::take_children(&item) {
            dom::insert_before(&item, &child);
        }
        dom::detach(&item);
    }
    for element in dom::select(&body, dom::is_element) {
        snap_colors(&element);
    }
    dom::inner_html(&body)
}

/// Colours: Slack's Lato text is black, an unreadable colour snaps to the theme's, and black (or
/// white) goes so the text takes the theme's; a `<font color>` that snaps to black goes.
fn snap_colors(element: &NodeRef) {
    let mut style = Style::of(element);
    if style.get("font-family").contains("Slack-Lato") {
        style.set("color", "#000", false);
    }
    if let Some(color) = colors::pasted_color(style.get("color")) {
        match color {
            Some(color) => style.set("color", &color, false),
            None => style.remove("color"),
        }
        if style.is_empty() {
            dom::remove_attr(element, "style");
        } else {
            style.apply(element);
        }
    }
    if dom::attr(element, "color").is_some_and(|color| colors::font_color_snaps_to_black(&color)) {
        dom::remove_attr(element, "color");
    }
}

/// Excel's styles: each rule of the sheet's first `<style>` is appended to the style of what it
/// selects. Excel writes simple selectors (`td`, `.xl65`, `br`), which are what's matched.
fn inline_sheet(document: &NodeRef, body: &NodeRef) {
    let Some(sheet) = dom::select_first(document, |node| dom::is(node, "style")) else {
        return;
    };
    let css = dom::text_content(&sheet)
        .replace("<!--", "")
        .replace("-->", "");
    for rule in css.split('}') {
        let Some((selectors, declarations)) = rule.split_once('{') else {
            continue;
        };
        let declarations = declarations.trim();
        if declarations.is_empty() || selectors.trim_start().starts_with('@') {
            continue;
        }
        for selector in selectors.split(',').map(str::trim) {
            let (tag, class) = selector
                .split_once('.')
                .map_or((selector, None), |(tag, class)| (tag, Some(class)));
            let simple = |part: &str| {
                part.chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
            };
            if selector.is_empty()
                || !simple(tag)
                || class.is_some_and(|class| class.is_empty() || !simple(class))
            {
                continue;
            }
            for element in dom::select(body, |node| {
                (tag.is_empty() || dom::is(node, &tag.to_ascii_lowercase()))
                    && class.is_none_or(|class| dom::has_class(node, class))
            }) {
                let existing = dom::attr(&element, "style").unwrap_or_default();
                let joined = if existing.trim().is_empty() {
                    declarations.to_owned()
                } else {
                    format!("{}; {declarations}", existing.trim().trim_end_matches(';'))
                };
                dom::set_attr(&element, "style", &joined);
            }
        }
    }
}

/// Word's list paragraphs (`[style*="mso-list:l"]`) rebuilt as lists: a run of adjacent ones is
/// one list, nested by each one's `level`, numbered when its bullet (`mso-list:Ignore`) has a
/// digit.
fn word_lists(body: &NodeRef) {
    let items = dom::select(body, |node| {
        dom::attr(node, "style").is_some_and(|style| style.contains("mso-list:l"))
    });
    let mut list: Option<(NodeRef, NodeRef, usize)> = None;
    for (index, item) in items.iter().enumerate() {
        let tag = if dom::select_first(item, is_bullet).is_some_and(|bullet| {
            dom::text_content(&bullet)
                .bytes()
                .any(|b| b.is_ascii_digit())
        }) {
            "ol"
        } else {
            "ul"
        };
        let (root, mut current, mut depth) = list.take().unwrap_or_else(|| {
            let root = dom::element(tag, &[]);
            (root.clone(), root, 0)
        });
        let level = level(item);
        while depth < level {
            let nested = dom::element("ul", &[]);
            dom::append(&current, &nested);
            current = nested;
            depth += 1;
        }
        while depth > level {
            current = dom::parent(&current).unwrap_or_else(|| root.clone());
            depth -= 1;
        }
        let root = if tag == "ol" && dom::is(&current, "ul") {
            let renamed = rename(&current, "ol");
            let root = if dom::key(&current) == dom::key(&root) {
                renamed.clone()
            } else {
                root
            };
            current = renamed;
            root
        } else {
            root
        };
        if let Some(bullet) = dom::select_first(item, is_bullet) {
            dom::detach(&bullet);
        }
        let entry = dom::element("li", &[]);
        for child in dom::take_children(item) {
            dom::append(&entry, &child);
        }
        dom::append(&current, &entry);
        let next = dom::following_siblings(item)
            .into_iter()
            .find(dom::is_element);
        if next.map(|next| dom::key(&next)) == items.get(index + 1).map(dom::key) {
            list = Some((root, current, depth));
        } else {
            dom::insert_before(item, &root);
        }
        dom::detach(item);
    }
}

/// A Word list paragraph's bullet or number (`[style*="Ignore"]`).
fn is_bullet(node: &NodeRef) -> bool {
    dom::attr(node, "style").is_some_and(|style| style.contains("Ignore"))
}

/// A Word list paragraph's depth: its `mso-list:l0 level2 lfo1` level, from 0.
fn level(item: &NodeRef) -> usize {
    let style = dom::attr(item, "style").unwrap_or_default();
    style
        .split("mso-list:")
        .nth(1)
        .and_then(|value| value.split(';').next())
        .and_then(|value| value.split_whitespace().nth(1))
        .and_then(|level| level.strip_prefix("level")?.parse::<usize>().ok())
        .map_or(0, |level| level.saturating_sub(1))
}

/// Blank lines between spaced paragraphs: an empty line after a paragraph (`div` or `p`, not
/// in a list or table, with something to show) whose margin with the next paragraph is 6px or
/// more; a `p` without a margin of its own has 16px when the page has no `<style>`.
fn blank_lines(body: &NodeRef, unstyled: bool) {
    let paragraph = |node: &NodeRef| dom::is(node, "div") || dom::is(node, "p");
    let visible = |node: &NodeRef| {
        !dom::text_content(node).trim().is_empty()
            || dom::select_first(node, |n| dom::is(n, "img")).is_some()
    };
    let margin = |node: &NodeRef, side: &str| {
        let value = Style::of(node).get(side).trim().to_ascii_lowercase();
        if value.is_empty() {
            return if dom::is(node, "p") && unstyled {
                16.0
            } else {
                0.0
            };
        }
        let split = value
            .find(|ch: char| ch.is_ascii_alphabetic())
            .unwrap_or(value.len());
        let factor = match &value[split..] {
            "px" => 1.0,
            "pt" => 4.0 / 3.0,
            "em" | "rem" => 16.0,
            _ => return 0.0,
        };
        value[..split]
            .parse::<f32>()
            .map_or(0.0, |number| number * factor)
    };
    for block in dom::select(body, paragraph) {
        let Some(next) = dom::following_siblings(&block)
            .into_iter()
            .find(dom::is_element)
        else {
            continue;
        };
        let listed = dom::closest(&block, |node| {
            ["li", "table", "ul", "ol"]
                .iter()
                .any(|tag| dom::is(node, tag))
        })
        .is_some();
        if !paragraph(&next)
            || listed
            || !visible(&block)
            || !visible(&next)
            || margin(&block, "margin-bottom").max(margin(&next, "margin-top")) < 6.0
        {
            continue;
        }
        let line = dom::element("div", &[]);
        dom::append(&line, &dom::element("br", &[]));
        dom::insert_after(&block, &line);
    }
}

/// Replaces `node` with a `tag` element holding its attributes and children.
fn rename(node: &NodeRef, tag: &str) -> NodeRef {
    let attributes = dom::attrs(node);
    let pairs: Vec<(&str, &str)> = attributes
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    let renamed = dom::element(tag, &pairs);
    for child in dom::take_children(node) {
        dom::append(&renamed, &child);
    }
    dom::insert_before(node, &renamed);
    dom::detach(node);
    renamed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_lists_become_lists() {
        let html = r#"<html xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:w="urn:schemas-microsoft-com:office:word"><body>
<p class=MsoListParagraph style='mso-list:l0 level1 lfo1'><span style='mso-list:Ignore'>1.</span>One</p>
<p class=MsoListParagraph style='mso-list:l0 level2 lfo1'><span style='mso-list:Ignore'>o</span>Inner</p>
<p class=MsoListParagraph style='mso-list:l0 level1 lfo1'><span style='mso-list:Ignore'>2.</span>Two</p>
<p>After</p></body></html>"#;
        let out = prepare_paste(html);
        assert!(
            out.contains("<ol><li>One</li><ul><li>Inner</li></ul><li>Two</li></ol>"),
            "{out}"
        );
        assert!(out.contains("<p>After</p>"), "{out}");
    }

    #[test]
    fn file_images_docs_wrappers_and_hidden_content_go() {
        let out = prepare_paste(
            r#"<b id="docs-internal-guid-1"><p>Hi<img src="file:///tmp/a.png"></p></b><div style="display: none">x</div><div style="display:inline">in</div>"#,
        );
        // The blank line comes first, from the hidden block's spacing.
        assert_eq!(
            out,
            r#"<p>Hi</p><div><br></div><span style="display:inline">in</span>"#
        );
    }

    #[test]
    fn spaced_paragraphs_keep_a_blank_line() {
        assert_eq!(
            prepare_paste("<p>One</p><p>Two</p>"),
            "<p>One</p><div><br></div><p>Two</p>"
        );
        assert_eq!(
            prepare_paste("<style></style><p>One</p><p>Two</p>"),
            "<p>One</p><p>Two</p>",
            "a styled page's paragraphs have no default margin"
        );
        assert_eq!(
            prepare_paste("<div>One</div><div style=\"margin-top: 1em\">Two</div>"),
            "<div>One</div><div><br></div><div style=\"margin-top: 1em\">Two</div>"
        );
    }

    #[test]
    fn colors_snap_for_the_theme() {
        assert_eq!(
            prepare_paste(
                r##"<span style="color: rgb(0, 0, 0)">a</span><span style="color: #fefefe">b</span><span style="color: #c00">c</span><font color="#000">d</font>"##
            ),
            r##"<span>a</span><span>b</span><span style="color: #c00">c</span><font>d</font>"##
        );
    }

    #[test]
    fn excel_rules_are_inlined() {
        let html = r#"<html xmlns:x="urn:schemas-microsoft-com:office:excel" xmlns:o="urn:schemas-microsoft-com:office:office"><head><style><!-- td {padding-top:1px} .xl65 {font-weight:700} --></style></head><body><table><tr><td class=xl65>B</td></tr></table></body></html>"#;
        assert!(
            prepare_paste(html)
                .contains(r#"<td class="xl65" style="padding-top:1px; font-weight:700">B</td>"#),
            "{}",
            prepare_paste(html)
        );
    }
}
