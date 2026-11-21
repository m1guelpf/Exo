//! HTML presentational attributes Blitz doesn't map to CSS yet (`cellpadding`, `cellspacing`,
//! `valign`, `<font>`, `background`), which email HTML leans on. Browsers apply them as author
//! rules with zero specificity that come before every stylesheet; the rules written here are
//! `:where()` rules placed first in the document, which is the same.

use std::{collections::HashSet, fmt::Write as _};

use crate::dom::{self, Style};

/// How many value-specific rules a body gets. Mail uses a handful of distinct values; a hostile
/// one could use thousands, and Blitz matches every rule against every element.
const MAX_RULES: usize = 256;

/// The rules for every presentational attribute in `html` (the first [`MAX_RULES`] values).
pub(crate) fn stylesheet(html: &str) -> String {
    let document = dom::parse(html);
    let mut css = String::from(STATIC_RULES);
    let mut seen = HashSet::new();
    let mut once = |kind: &'static str, value: &str| {
        seen.len() < MAX_RULES && seen.insert((kind, value.to_owned()))
    };
    for element in dom::select(&document, dom::is_element) {
        let tag = dom::tag(&element).unwrap_or_default().to_ascii_lowercase();
        let attr = |name: &str| {
            dom::attr(&element, name)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        if tag == "table" {
            if let Some(value) = attr("cellspacing")
                && let Some(px) = pixels(&value)
                && once("cellspacing", &value)
            {
                let _ = writeln!(
                    css,
                    ":where(table[cellspacing=\"{}\"]) {{ border-spacing: {px}px; }}",
                    escape(&value)
                );
            }
            if let Some(value) = attr("cellpadding")
                && let Some(px) = pixels(&value)
                && once("cellpadding", &value)
            {
                let table = format!("table[cellpadding=\"{}\"]", escape(&value));
                let _ = writeln!(
                    css,
                    ":where({table} > * > tr > :is(td, th), {table} > tr > :is(td, th)) {{ padding: {px}px; }}"
                );
            }
        }
        if tag == "font" {
            if let Some(value) = attr("color")
                && once("color", &value)
            {
                let color = if value.len() == 6 && value.chars().all(|c| c.is_ascii_hexdigit()) {
                    format!("#{value}")
                } else {
                    sanitize_value(&value)
                };
                let _ = writeln!(
                    css,
                    ":where(font[color=\"{}\"]) {{ color: {color}; }}",
                    escape(&value)
                );
            }
            if let Some(value) = attr("face")
                && once("face", &value)
            {
                let families: Vec<String> = value
                    .split(',')
                    .map(|family| family.trim().trim_matches(['"', '\'']))
                    .filter(|family| !family.is_empty())
                    .map(|family| {
                        if GENERIC_FAMILIES.contains(&family.to_ascii_lowercase().as_str()) {
                            family.to_ascii_lowercase()
                        } else {
                            format!("\"{}\"", escape(family))
                        }
                    })
                    .collect();
                let _ = writeln!(
                    css,
                    ":where(font[face=\"{}\"]) {{ font-family: {}; }}",
                    escape(&value),
                    families.join(", ")
                );
            }
            if let Some(value) = attr("size")
                && let Some(size) = legacy_font_size(&value)
                && once("size", &value)
            {
                let _ = writeln!(
                    css,
                    ":where(font[size=\"{}\"]) {{ font-size: {size}; }}",
                    escape(&value)
                );
            }
        }
        if let Some(style) = attr("style")
            && let Some(size) = optical_size(&style)
            && once("opsz", &style)
        {
            let _ = writeln!(
                css,
                ":where([style=\"{}\"]) {{ font-variation-settings: \"opsz\" {size}; }}",
                escape(&style)
            );
        }
        if matches!(
            tag.as_str(),
            "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th"
        ) && let Some(value) = attr("background")
            && once("background", &value)
        {
            let _ = writeln!(
                css,
                ":where({tag}[background=\"{}\"]) {{ background-image: url(\"{}\"); }}",
                escape(&value),
                escape(&value)
            );
        }
    }
    css
}

const GENERIC_FAMILIES: &[&str] = &[
    "serif",
    "sans-serif",
    "monospace",
    "cursive",
    "fantasy",
    "system-ui",
];

/// Hints with a fixed set of values.
const STATIC_RULES: &str = "\
:where(:is(thead, tbody, tfoot, tr, td, th, col, colgroup)[valign=\"top\" i]) { vertical-align: top; }
:where(:is(thead, tbody, tfoot, tr, td, th, col, colgroup)[valign=\"middle\" i]) { vertical-align: middle; }
:where(:is(thead, tbody, tfoot, tr, td, th, col, colgroup)[valign=\"bottom\" i]) { vertical-align: bottom; }
:where(:is(thead, tbody, tfoot, tr, td, th, col, colgroup)[valign=\"baseline\" i]) { vertical-align: baseline; }
:where(img[align=\"left\" i]) { float: left; }
:where(img[align=\"right\" i]) { float: right; }
:where(img[align=\"middle\" i], img[align=\"center\" i]) { vertical-align: middle; }
:where(img[align=\"top\" i]) { vertical-align: top; }
:where(img[align=\"bottom\" i]) { vertical-align: bottom; }
";

/// The optical size Chromium would give text styled `style` (its font size, `font-optical-sizing:
/// auto`), when that's above the 17 `body.css` defaults to. Only inline px sizes are seen.
fn optical_size(style: &str) -> Option<f32> {
    let size: f32 = Style::parse(style)
        .get("font-size")
        .strip_suffix("px")?
        .trim()
        .parse()
        .ok()?;
    (size > 17.0).then_some(size)
}

/// A non-negative integer attribute (`cellspacing="4"`, `"4px"`).
fn pixels(value: &str) -> Option<u32> {
    let digits: String = value.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// `<font size>`: 1–7, or relative to 3 with a sign.
fn legacy_font_size(value: &str) -> Option<&'static str> {
    let (sign, rest) = match value.chars().next()? {
        '+' => (1, &value[1..]),
        '-' => (-1, &value[1..]),
        _ => (0, value),
    };
    let number: i32 = rest
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()?;
    let size = if sign == 0 { number } else { 3 + sign * number };
    Some(match size.clamp(1, 7) {
        1 => "x-small",
        2 => "small",
        3 => "medium",
        4 => "large",
        5 => "x-large",
        6 => "xx-large",
        _ => "xxx-large",
    })
}

/// Escapes text for a double-quoted CSS string, `<` included so it can't close the `<style>`.
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\a ")
        .replace('<', "\\3c ")
}

/// A property value that can't break out of its declaration.
fn sanitize_value(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '{' | '}' | ';' | '<' | '>'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_table_and_font_attributes() {
        let css = stylesheet(
            "<table cellspacing=0 cellpadding=\"8\"><tr><td><font color=\"ff0000\" size=\"+1\" face=\"Arial, sans-serif\">x</font></td></tr></table>",
        );
        assert!(css.contains(":where(table[cellspacing=\"0\"]) { border-spacing: 0px; }"));
        assert!(css.contains("{ padding: 8px; }"));
        assert!(css.contains(":where(font[color=\"ff0000\"]) { color: #ff0000; }"));
        assert!(css.contains(":where(font[size=\"+1\"]) { font-size: large; }"));
        assert!(css.contains("font-family: \"Arial\", sans-serif;"));
    }

    #[test]
    fn caps_the_rules_a_body_gets() {
        let html: String = (0..1000)
            .map(|i| format!("<table cellpadding=\"{i}\"><tr><td>x</td></tr></table>"))
            .collect();
        let rules = stylesheet(&html).lines().count() - STATIC_RULES.lines().count();
        assert_eq!(rules, MAX_RULES);
    }

    #[test]
    fn values_cannot_close_the_stylesheet() {
        let css = stylesheet("<font face=\"a</style><img src=x>\">x</font>");
        assert!(!css.contains('<'), "{css}");
    }
}
