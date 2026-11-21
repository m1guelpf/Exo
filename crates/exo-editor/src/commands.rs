//! The editor's formatting shortcuts, as data the app registers in its command registry
//! (`exo-commands`, which dispatches keys before anything draws).

use crate::{Editor, Format, LinkAction};

/// A formatting command with a key: the body's keymap and its help-sheet copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FormatCommand {
    /// ⌘B.
    Bold,
    /// ⌘I.
    Italic,
    /// ⌘U.
    Underline,
    /// ⌘K: the link input, or removing the link the selection is in.
    Hyperlink,
    /// ⌘O: the colour subformatter.
    Color,
    /// ⌘⇧X.
    Strikethrough,
    /// ⌘⇧7.
    Numbers,
    /// ⌘⇧8.
    Bullets,
    /// ⌘⇧9.
    Quote,
    /// Tab in a list.
    IndentList,
    /// Shift+Tab in a list.
    OutdentList,
    /// ⌘].
    IncreaseIndent,
    /// ⌘[.
    DecreaseIndent,
}

impl FormatCommand {
    /// Every command, in the help sheet's order.
    pub const ALL: [Self; 13] = [
        Self::Bold,
        Self::Italic,
        Self::Underline,
        Self::Hyperlink,
        Self::Color,
        Self::Strikethrough,
        Self::Numbers,
        Self::Bullets,
        Self::Quote,
        Self::IndentList,
        Self::OutdentList,
        Self::IncreaseIndent,
        Self::DecreaseIndent,
    ];

    /// A stable id for the registry.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Bold => "EDITOR_BOLD",
            Self::Italic => "EDITOR_ITALIC",
            Self::Underline => "EDITOR_UNDERLINE",
            Self::Hyperlink => "EDITOR_HYPERLINK",
            Self::Color => "EDITOR_COLOR",
            Self::Strikethrough => "EDITOR_STRIKETHROUGH",
            Self::Numbers => "EDITOR_NUMBERS",
            Self::Bullets => "EDITOR_BULLETS",
            Self::Quote => "EDITOR_QUOTE",
            Self::IndentList => "EDITOR_INDENT_LIST",
            Self::OutdentList => "EDITOR_OUTDENT_LIST",
            Self::IncreaseIndent => "EDITOR_INCREASE_INDENT",
            Self::DecreaseIndent => "EDITOR_DECREASE_INDENT",
        }
    }

    /// The help sheet's label for it (its "format" section).
    pub const fn title(self) -> &'static str {
        match self {
            Self::Bold => "Bold",
            Self::Italic => "Italics",
            Self::Underline => "Underline",
            Self::Hyperlink => "Hyperlink",
            Self::Color => "Color",
            Self::Strikethrough => "Strikethrough",
            Self::Numbers => "Numbers",
            Self::Bullets => "Bullets",
            Self::Quote => "Quote",
            Self::IndentList => "Indent List",
            Self::OutdentList => "Outdent List",
            Self::IncreaseIndent => "Increase Indent",
            Self::DecreaseIndent => "Decrease Indent",
        }
    }

    /// Its keys, in Mousetrap notation (`mod` is ⌘ on a Mac).
    pub const fn keys(self) -> &'static [&'static str] {
        match self {
            Self::Bold => &["mod+b"],
            Self::Italic => &["mod+i"],
            Self::Underline => &["mod+u"],
            Self::Hyperlink => &["mod+k"],
            Self::Color => &["mod+o"],
            Self::Strikethrough => &["mod+shift+x"],
            Self::Numbers => &["mod+shift+7"],
            Self::Bullets => &["mod+shift+8"],
            Self::Quote => &["mod+shift+9"],
            Self::IndentList => &["tab"],
            Self::OutdentList => &["shift+tab"],
            Self::IncreaseIndent => &["mod+]"],
            Self::DecreaseIndent => &["mod+["],
        }
    }

    /// Whether the editor takes the key now, so the app registers it only then: ⌘K only with a
    /// selection or in a link (otherwise it opens the palette), Tab and Shift+Tab only in a list
    /// (otherwise they move focus).
    pub fn applies(self, editor: &Editor) -> bool {
        match self {
            Self::Hyperlink => editor.link_action() != LinkAction::None,
            Self::IndentList | Self::OutdentList => editor.in_list(),
            _ => true,
        }
    }

    /// Runs it.
    pub fn run(self, editor: &mut Editor) {
        match self {
            Self::Bold => editor.toggle_format(Format::Bold),
            Self::Italic => editor.toggle_format(Format::Italic),
            Self::Underline => editor.toggle_format(Format::Underline),
            Self::Strikethrough => editor.toggle_format(Format::Strikethrough),
            Self::Hyperlink => editor.begin_link(),
            Self::Color => editor.begin_color(),
            Self::Numbers => editor.toggle_list(true),
            Self::Bullets => editor.toggle_list(false),
            Self::Quote => editor.toggle_quote(),
            Self::IndentList => {
                editor.indent_list();
            }
            Self::OutdentList => {
                editor.outdent_list();
            }
            Self::IncreaseIndent => editor.increase_level(),
            Self::DecreaseIndent => editor.decrease_level(),
        }
    }
}
