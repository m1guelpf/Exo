//! The one `Command` type behind both a keyboard shortcut and a palette row.

use std::borrow::Cow;

use crate::{
    keys::display_tokens,
    region::{Region, Scope},
};

type Text = Cow<'static, str>;

/// Something the user can do: bound to keys, listed in the ⌘K palette, or both.
///
/// A command with a title is a palette row; one with keys is a shortcut. They share one
/// definition (title, keys, scope, action), so a shortcut and its row can't disagree. `A` is the
/// app's action type: running the command hands back a clone of `action`.
#[derive(Clone, Debug)]
#[must_use]
pub struct Command<A> {
    pub(crate) id: Text,
    pub(crate) title: Option<Text>,
    pub(crate) action: A,
    pub(crate) keys: Vec<Text>,
    pub(crate) bind_keys: bool,
    /// A backtick stands in for an `esc` binding.
    pub(crate) backtick_escape: bool,
    /// Its keys only press it when they type this.
    pub(crate) typing: Option<&'static str>,
    pub(crate) key_code: Option<Vec<Text>>,
    pub(crate) scope: Scope,
    pub(crate) within_palette: bool,
    pub(crate) repeat: Option<A>,
    pub(crate) release: Option<A>,
    pub(crate) aliases: Vec<Text>,
    pub(crate) exclude_alias_from_title: Vec<Text>,
    pub(crate) group_include_aliases: Vec<Text>,
    pub(crate) operative_word: Option<Text>,
    pub(crate) default_score: f64,
    pub(crate) scale: Option<f64>,
    pub(crate) follow: Vec<Text>,
    pub(crate) multiple: bool,
    pub(crate) icon: Option<&'static str>,
    /// An account's avatar in the icon's place.
    pub(crate) avatar: Option<Avatar>,
    pub(crate) description: Option<Text>,
    pub(crate) side_title: Option<Text>,
    pub(crate) in_use: bool,
    pub(crate) query_only: bool,
    /// Run from the palette with ⌘ held (Ctrl elsewhere), the row leaves it open.
    pub(crate) stays_open_on_command: bool,
}

/// An account's avatar in a row: its initial on its color, or its profile photo in a ring of
/// that color.
#[derive(Clone, Debug)]
pub(crate) struct Avatar {
    pub initial: String,
    pub fill: egui::Color32,
    pub photo: Option<exo_ui::AvatarPhoto>,
}

impl<A> Command<A> {
    /// A palette command: `id` identifies it (`MARK_DONE`; deduplication, `follow`), `title` is
    /// what its row reads. Live everywhere until scoped with [`Self::regions`].
    pub fn new(id: impl Into<Text>, title: impl Into<Text>, action: A) -> Self {
        Self {
            title: Some(title.into()),
            ..Self::shortcut(id, action)
        }
    }

    /// A keyboard-only command (no palette row), such as moving the list cursor.
    pub fn shortcut(id: impl Into<Text>, action: A) -> Self {
        Self {
            id: id.into(),
            title: None,
            action,
            keys: Vec::new(),
            bind_keys: true,
            backtick_escape: true,
            typing: None,
            key_code: None,
            scope: Scope::Global,
            within_palette: false,
            repeat: None,
            release: None,
            aliases: Vec::new(),
            exclude_alias_from_title: Vec::new(),
            group_include_aliases: Vec::new(),
            operative_word: None,
            default_score: 0.0,
            scale: None,
            follow: Vec::new(),
            multiple: false,
            icon: None,
            avatar: None,
            description: None,
            side_title: None,
            in_use: false,
            query_only: false,
            stays_open_on_command: false,
        }
    }

    /// Binds `binding`, in Mousetrap's notation: `e`, `shift+e`, `mod+k` (`mod` is ⌘ on a Mac, Ctrl
    /// elsewhere; `ctralt` is Ctrl on a Mac, Alt elsewhere), `#` (shifted symbols are Shift plus
    /// their US-layout key) or `g i` (a sequence). Call again for alternatives (`j` and `down`).
    /// The first binding is what the palette row shows, unless [`Self::key_code`] says otherwise.
    pub fn key(mut self, binding: impl Into<Text>) -> Self {
        self.keys.push(binding.into());
        self
    }

    /// Binds every binding in `bindings`.
    pub fn keys<I: IntoIterator<Item = S>, S: Into<Text>>(mut self, bindings: I) -> Self {
        self.keys.extend(bindings.into_iter().map(Into::into));
        self
    }

    /// Shows `tokens` as the row's keycaps instead of the first binding (`["⌘", "O"]`, or
    /// `["g", "then", "i"]`).
    pub fn key_code<I: IntoIterator<Item = S>, S: Into<Text>>(mut self, tokens: I) -> Self {
        self.key_code = Some(tokens.into_iter().map(Into::into).collect());
        self
    }

    /// Shows the keys on the row without binding them, for keys another registration handles.
    pub fn display_only(mut self) -> Self {
        self.bind_keys = false;
        self
    }

    /// Whether a backtick presses this command's `esc` binding, as it does by default. Compose's
    /// own Escape answers to it only with Backtick as Escape on; in the body, the editor keeps a
    /// backtick to type it.
    pub fn backtick_escape(mut self, on: bool) -> Self {
        self.backtick_escape = on;
        self
    }

    /// Its keys press it only when they type `text` on the keyboard's layout: Trash's `\` and
    /// `alt+3` on UK keyboards, which type `#` there.
    pub fn when_typing(mut self, text: &'static str) -> Self {
        self.typing = Some(text);
        self
    }

    /// Makes the keys work while the palette is open too.
    pub fn within_palette(mut self) -> Self {
        self.within_palette = true;
        self
    }

    /// Live only while one of `regions` is active (for keys) or was active when the palette
    /// opened (for rows).
    pub fn regions(mut self, regions: &'static [Region]) -> Self {
        self.scope = Scope::In(regions);
        self
    }

    /// Holding the key fires `repeat` after 250ms and then every 65ms, instead of nothing. The
    /// first press still fires the command's action.
    pub fn repeat(mut self, repeat: A) -> Self {
        self.repeat = Some(repeat);
        self
    }

    /// Fires `release` when a held repeating key is let go.
    pub fn on_release(mut self, release: A) -> Self {
        self.release = Some(release);
        self
    }

    /// Search synonyms; when one wins, the row reads `Title (Alias)`.
    pub fn aliases<I: IntoIterator<Item = S>, S: Into<Text>>(mut self, aliases: I) -> Self {
        self.aliases.extend(aliases.into_iter().map(Into::into));
        self
    }

    /// Aliases that match without being appended to the title.
    pub fn exclude_alias_from_title<I: IntoIterator<Item = S>, S: Into<Text>>(
        mut self,
        aliases: I,
    ) -> Self {
        self.exclude_alias_from_title
            .extend(aliases.into_iter().map(Into::into));
        self
    }

    /// Fallback synonyms, scored at half weight when nothing else matches.
    pub fn group_include_aliases<I: IntoIterator<Item = S>, S: Into<Text>>(
        mut self,
        aliases: I,
    ) -> Self {
        self.group_include_aliases
            .extend(aliases.into_iter().map(Into::into));
        self
    }

    /// A keyword that boosts the row when typed (averaged with the title's score, doubled on an
    /// exact match).
    pub fn operative_word(mut self, word: impl Into<Text>) -> Self {
        self.operative_word = Some(word.into());
        self
    }

    /// Rank with an empty query (higher first); rows without one are hidden until something is
    /// typed. Also breaks ties.
    pub fn default_score(mut self, score: f64) -> Self {
        self.default_score = score;
        self
    }

    /// Multiplies the row's score.
    pub fn scale(mut self, scale: f64) -> Self {
        self.scale = Some(scale);
        self
    }

    /// Always lists this row right after the command `id` when both match.
    pub fn follow(mut self, id: impl Into<Text>) -> Self {
        self.follow.push(id.into());
        self
    }

    /// Keeps the row when several conversations are selected.
    pub fn multiple(mut self) -> Self {
        self.multiple = true;
        self
    }

    /// The row's icon, an [`exo_ui::paint_icon`] name: one of the white data-URI glyphs
    /// (`data/done`).
    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// An account's avatar in the icon's place, as the Accounts palette's rows have: its
    /// profile `photo` in a ring of `fill`, or without one, `initial` on a `fill` circle.
    pub fn avatar(
        mut self,
        initial: impl Into<String>,
        fill: egui::Color32,
        photo: Option<exo_ui::AvatarPhoto>,
    ) -> Self {
        self.avatar = Some(Avatar {
            initial: initial.into(),
            fill,
            photo,
        });
        self
    }

    /// Grey text after the title.
    pub fn description(mut self, description: impl Into<Text>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Text on the right instead of keycaps (a date in the Remind me palette).
    pub fn side_title(mut self, side_title: impl Into<Text>) -> Self {
        self.side_title = Some(side_title.into());
        self
    }

    /// Shows a ✓ on the right (the current theme, a label every selected conversation has).
    pub fn in_use(mut self) -> Self {
        self.in_use = true;
        self
    }

    /// Lists the row only once something is typed. The `go to …` rows do this while a conversation
    /// is highlighted, so they don't crowd its actions out of the empty palette.
    pub fn query_only(mut self) -> Self {
        self.query_only = true;
        self
    }

    /// Leaves the palette open when the row runs with ⌘ held (Ctrl elsewhere), as Open Links'
    /// link rows do. What was held is [`crate::Commands::ran_with`].
    pub fn stays_open_on_command(mut self) -> Self {
        self.stays_open_on_command = true;
        self
    }

    /// The same command running `f(action)` instead.
    pub(crate) fn map<B>(self, f: impl Fn(A) -> B) -> Command<B> {
        Command {
            id: self.id,
            title: self.title,
            action: f(self.action),
            keys: self.keys,
            bind_keys: self.bind_keys,
            backtick_escape: self.backtick_escape,
            typing: self.typing,
            key_code: self.key_code,
            scope: self.scope,
            within_palette: self.within_palette,
            repeat: self.repeat.map(&f),
            release: self.release.map(&f),
            aliases: self.aliases,
            exclude_alias_from_title: self.exclude_alias_from_title,
            group_include_aliases: self.group_include_aliases,
            operative_word: self.operative_word,
            default_score: self.default_score,
            scale: self.scale,
            follow: self.follow,
            multiple: self.multiple,
            icon: self.icon,
            avatar: self.avatar,
            description: self.description,
            side_title: self.side_title,
            in_use: self.in_use,
            query_only: self.query_only,
            stays_open_on_command: self.stays_open_on_command,
        }
    }

    /// The command's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The row's title, if it's a palette command.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// The keycap tokens the row shows: [`Self::key_code`], else the first binding split on `+`.
    pub(crate) fn display_keys(&self) -> Vec<&str> {
        match (&self.key_code, self.keys.first()) {
            (Some(tokens), _) => tokens.iter().map(AsRef::as_ref).collect(),
            (None, Some(binding)) => display_tokens(binding),
            (None, None) => Vec::new(),
        }
    }
}
