//! Themes and their colors.
//!
//! Exo draws one style, Classic, in two themes: Snow (light) and Carbon (dark). Classic's colors
//! are literals, one per component and theme. [`Palette`] names the roles those literals play most
//! often; a component whose color differs from its role keeps the literal at its call site, saying
//! where it applies.

use egui::{Color32, Context, Id};

/// The theme components draw in, after resolving Match System.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Theme {
    /// `snow`, Classic light, and what Match System shows when the system doesn't say.
    #[default]
    Snow,
    /// `carbon`, Classic dark.
    Carbon,
}

impl Theme {
    /// This theme's colors.
    pub const fn palette(self) -> &'static Palette {
        match self {
            Self::Snow => &SNOW,
            Self::Carbon => &CARBON,
        }
    }

    /// The theme `auto` (Match System) resolves to.
    pub const fn matching(system: egui::Theme) -> Self {
        match system {
            egui::Theme::Light => Self::Snow,
            egui::Theme::Dark => Self::Carbon,
        }
    }
}

fn theme_id() -> Id {
    Id::new("exo_ui::theme")
}

/// Makes `theme` the one components draw with.
pub fn set_theme(ctx: &Context, theme: Theme) {
    ctx.data_mut(|data| data.insert_temp(theme_id(), theme));
}

/// The theme components draw with.
pub fn theme(ctx: &Context) -> Theme {
    ctx.data(|data| data.get_temp(theme_id()))
        .unwrap_or_default()
}

/// The current theme's colors.
pub fn palette(ctx: &Context) -> &'static Palette {
    theme(ctx).palette()
}

fn app_theme_id() -> Id {
    Id::new("exo_ui::app_theme")
}

/// The app's theme, even inside a forced-theme island ([`with_theme`]): the few colors a Carbon
/// dialog doesn't take from Snow (a disabled primary button's fill, for one) read it.
pub fn app_theme(ctx: &Context) -> Theme {
    ctx.data(|data| data.get_temp(app_theme_id()))
        .unwrap_or_else(|| theme(ctx))
}

/// Runs `paint` with `theme` as the theme, then restores the current one: a forced-theme island,
/// like Carbon's dialogs, which keep Snow's colors on a light card.
pub fn with_theme<R>(ctx: &Context, theme: Theme, paint: impl FnOnce() -> R) -> R {
    let current = self::theme(ctx);
    let outermost = ctx
        .data(|data| data.get_temp::<Theme>(app_theme_id()))
        .is_none();
    if outermost {
        ctx.data_mut(|data| data.insert_temp(app_theme_id(), current));
    }
    set_theme(ctx, theme);
    let result = paint();
    set_theme(ctx, current);
    if outermost {
        ctx.data_mut(|data| data.remove::<Theme>(app_theme_id()));
    }
    result
}

macro_rules! roles {
    ($($(#[$doc:meta])* $field:ident = $snow:literal, $carbon:literal;)*) => {
        /// Classic's colors for one theme, by role, each documented with where it applies.
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct Palette {
            /// The theme these are the colors of, for the few components whose Classic rules
            /// differ from the roles here.
            pub theme: Theme,
            $($(#[$doc])* pub $field: Color32,)*
        }

        const SNOW: Palette = Palette { theme: Theme::Snow, $($field: css_color($snow),)* };
        const CARBON: Palette = Palette { theme: Theme::Carbon, $($field: css_color($carbon),)* };
    };
}

roles! {
    /// App background.
    background_base_default = "#ffffff", "#27292d";
    /// Right pane background, `hsl(216, 100%, 99%)`.
    background_base_accent = "#fafcff", "#18191a";
    /// Raised cards: an expanded message.
    background_elevated_default = "#ffffff", "#363a40";
    /// Attachment cards.
    background_elevated_accent = "#dce2ea", "#555b64";
    /// Calendar invite cards.
    fill_base_accent = "#fafafa", "#2f3237";
    /// An invite's RSVP buttons' text.
    button_ghost_content_default = "rgba(0, 0, 0, .35)", "rgba(255, 255, 255, .45)";
    /// The accent (periwinkle): toggles, primary actions.
    background_brand_default = "#aeb1dd", "#aeb1dd";
    /// Hairlines: a dialog's rules.
    border_base_subdued = "rgba(0, 0, 0, .1)", "rgba(255, 255, 255, .1)";
    /// Focus underlines and bars: the list cursor's bar.
    border_base_active = "#aeb1dd", "#aeb1dd";
    /// A selected tab's underline.
    border_selected_active = "#aeb1dd", "#aeb1dd";
    /// Text on a colored fill: a label chip.
    text_base_inverse = "#ffffff", "#27292d";
    /// A starred conversation's dot.
    actions_star = "#ffcb33", "#ffcb33";
    /// A returned reminder's dot.
    actions_reminder_default = "#ce9ece", "#ce9ece";
    /// Keyboard focus outlines.
    border_focus_default = "#aeb1dd", "#aeb1dd";
    /// Primary text: a row's subject.
    text_base_default = "rgba(0, 0, 0, .9)", "rgba(255, 255, 255, .9)";
    /// Secondary text: a row's snippet and time.
    text_base_subdued = "rgba(0, 0, 0, .5)", "rgba(255, 255, 255, .5)";
    /// The accent as text and focus underlines: a focused dropdown's value.
    text_brand_default = "#aeb1dd", "#aeb1dd";
    /// Text on photos and dark surfaces.
    text_light_default = "#ffffff", "#ffffff";
    /// Text selection: `::selection`.
    text_selection_default = "#4198d233", "#4198d266";
    /// Icons: legacy icons are black (white in Carbon) at 30%.
    icon_base_default = "rgba(0, 0, 0, .3)", "rgba(255, 255, 255, .3)";
    /// De-emphasized icons.
    icon_base_subdued = "rgba(0, 0, 0, .2)", "rgba(255, 255, 255, .2)";
    /// Pressed controls: a picker's selected item.
    fill_active_default = "#f2f4fa", "#464b53";
    /// Hovered rows: a highlighted contact suggestion, `hsl(220, 45%, 97%)`.
    fill_hover_default = "#f4f6fb", "#464b53";
    /// Selected rows in the list.
    select_fill_default = "#54acdc", "rgba(57, 97, 132, .4)";
    /// Icons on photos.
    icon_light_default = "#ffffff", "#ffffff";
    /// Icons on the white fills drawn over photos.
    icon_dark_default = "#000000", "#000000";
    /// Text on white in either theme (a marketing message's card in Carbon).
    text_dark_default = "rgba(0, 0, 0, .9)", "rgba(0, 0, 0, .9)";
    /// The ⌘K palette.
    background_dark_default = "#1a1a1a", "#686d77";
    /// Drawers over the list: the label switcher.
    background_elevated_subdued = "#ffffff", "#18191a";
    /// Links in the folder drawer.
    vbar_anchor_default = "#54acdc", "#54acdc";
    /// Keycaps in palette rows and tooltips.
    background_key_default = "rgba(255, 255, 255, .15)", "#555b64";
    /// Emphasized keycaps: the shortcuts sheet's matches.
    background_key_active = "#8d9bad", "#8d9bad";
    /// Tooltips.
    tooltip_background_default = "#45494e", "#f5f5f5";
    /// Secondary text on dark surfaces: the palette's input hint.
    text_light_subdued = "rgba(255, 255, 255, .5)", "rgba(255, 255, 255, .5)";
    /// Hovered icons: legacy icons at 50%.
    icon_base_hover = "rgba(0, 0, 0, .5)", "rgba(255, 255, 255, .5)";
    /// Links and toast actions.
    link_text_primary = "#54acdc", "#54acdc";
    /// The inset bar of informational toasts.
    link_border_primary = "#54acdc", "#54acdc";
    /// Dividers: a dialog field's underline.
    border_base_default = "rgba(0, 0, 0, .1)", "rgba(255, 255, 255, .1)";
    /// The inset bar of error toasts.
    border_critical_default = "#ef8e8e", "#ef8e8e";
    /// The accent as a border: focused fields' underlines (the event form's).
    border_brand_default = "#aeb1dd", "#aeb1dd";
    /// The inset bar of Mark Done toasts.
    border_success_default = "#6ac692", "#6ac692";
    /// The current app in the rail.
    menu_icon_hover = "#4c4c4c", "#c3c4c6";
    /// Tertiary text: the list's separators, placeholders.
    text_base_accent = "rgba(0, 0, 0, .35)", "rgba(255, 255, 255, .35)";
    /// The list cursor's background, `hsl(220, 45%, 97%)`.
    background_brand_subdued = "#f4f6fb", "#464b53";
    /// The `Draft` marker in thread rows.
    actions_draft = "#6ebd77", "#6ebd77";
    /// Unread dots, `rgb(174, 177, 221)`.
    actions_unread = "#aeb1dd", "#aeb1dd";
    /// Behind the words a search matched: `hsl(47, 100%, 93%)` / `hsl(47, 30%, 30%)`.
    highlight_pro_default = "#fff7db", "#635a36";
    /// The check on a done thread's row.
    actions_done_default = "#8892b9", "#8892b9";
    /// That check, hovered.
    actions_done_hover = "#5e47e5", "#5e47e5";
    /// Contact pane section icons.
    icon_social_default = "#bfc8d2", "#bfc8d2";
    /// Icons on selected rows: a row's attachment icon.
    select_content_default = "#ffffff", "#ffffff";
    /// A checked checkbox.
    fill_activated_accent = "#aeb1dd", "#aeb1dd";
    /// Errors: a dialog field's error.
    text_critical_default = "#ef8e8e", "#ef8e8e";
    /// Quiet fills (a dragged row).
    background_base_subdued = "#fafafa", "#2f3237";
    /// The calendar grid's lines.
    border_base_muted = "rgba(0, 0, 0, .085)", "rgba(255, 255, 255, .085)";
    /// A radio's ring background.
    fill_base_default = "#ffffff", "#27292d";
    /// A declined attendee's badge.
    actions_reject = "#da8d82", "#da8d82";
    /// A selected RSVP button.
    button_ghost_fill_active = "rgba(84, 172, 220, .12)", "rgba(84, 172, 220, .15)";
    /// The glyph on an attendee's answer badge.
    icon_base_inverse = "#ffffff", "#27292d";
    /// Behind today in the calendar.
    background_calendar_today = "#aeb1dd", "#aeb1dd";
    /// Primary buttons: a dialog's primary action.
    button_primary_fill_default = "#aeb1dd", "#aeb1dd";
    /// Primary buttons, hovered (a dialog's adds a glow instead).
    button_primary_fill_hover = "#aeb1dd", "#aeb1dd";
    /// Primary buttons, pressed.
    button_primary_fill_active = "#aeb1dd", "#aeb1dd";
    /// Primary buttons' text: `hsla(0, 0%, 100%, .98)`.
    button_primary_content_default = "rgba(255, 255, 255, .98)", "rgba(255, 255, 255, .98)";
}

impl Palette {
    /// Classic's commonest literal: black at `snow` opacity in Snow, white at `carbon` in Carbon
    /// (`rgba(0, 0, 0, a)` in Snow, `hsla(0, 0%, 100%, a)` in Carbon).
    pub fn ink(&self, snow: f32, carbon: f32) -> Color32 {
        let alpha = |opacity: f32| (opacity * 255.0).round() as u8;
        match self.theme {
            Theme::Snow => Color32::from_black_alpha(alpha(snow)),
            Theme::Carbon => Color32::from_white_alpha(alpha(carbon)),
        }
    }
}

/// Parses `#rrggbb`, `#rrggbbaa`, `white`, or `rgb(r, g, b)` / `rgba(r, g, b, a)` (channels may
/// have decimals) at compile time.
pub(crate) const fn css_color(css: &str) -> Color32 {
    let bytes = css.as_bytes();
    if bytes[0] == b'#' {
        return css_hex(css);
    }
    if bytes.len() == 5 && bytes[0] == b'w' {
        return Color32::WHITE;
    }
    let mut at = if bytes.len() > 5 && bytes[3] == b'a' && bytes[4] == b'(' {
        5
    } else {
        assert!(
            bytes.len() > 4 && bytes[0] == b'r' && bytes[3] == b'(',
            "expected rgb(…), rgba(…), white or a hex color"
        );
        4
    };
    // Each number as ten-thousandths, rounded at the end.
    let mut values = [0u32, 0, 0, SCALE];
    let mut index = 0;
    while bytes[at] != b')' {
        while bytes[at] == b' ' || bytes[at] == b',' {
            at += 1;
        }
        let (mut whole, mut fraction, mut place, mut in_fraction) = (0u32, 0u32, SCALE, false);
        while bytes[at] != b',' && bytes[at] != b')' && bytes[at] != b' ' {
            if bytes[at] == b'.' {
                in_fraction = true;
            } else if in_fraction {
                if place > 1 {
                    place /= 10;
                    fraction += (bytes[at] - b'0') as u32 * place;
                }
            } else {
                whole = whole * 10 + (bytes[at] - b'0') as u32;
            }
            at += 1;
        }
        values[index] = whole * SCALE + fraction;
        index += 1;
        while bytes[at] == b' ' {
            at += 1;
        }
    }
    let alpha = ((values[3] * 255 + SCALE / 2) / SCALE) as u8;
    Color32::from_rgba_unmultiplied_const(
        rounded(values[0]),
        rounded(values[1]),
        rounded(values[2]),
        alpha,
    )
}

/// A channel in [`css_color`]'s fixed point, rounded to a byte.
const fn rounded(value: u32) -> u8 {
    ((value + SCALE / 2) / SCALE) as u8
}

/// [`css_color`]'s fixed point: ten-thousandths.
const SCALE: u32 = 10_000;

/// Parses `#rrggbb` or `#rrggbbaa` at compile time.
const fn css_hex(css: &str) -> Color32 {
    let bytes = css.as_bytes();
    assert!(
        bytes[0] == b'#' && (bytes.len() == 7 || bytes.len() == 9),
        "expected #rrggbb or #rrggbbaa"
    );
    let alpha = if bytes.len() == 9 {
        channel(bytes, 7)
    } else {
        255
    };
    Color32::from_rgba_unmultiplied_const(
        channel(bytes, 1),
        channel(bytes, 3),
        channel(bytes, 5),
        alpha,
    )
}

const fn channel(bytes: &[u8], at: usize) -> u8 {
    nibble(bytes[at]) * 16 + nibble(bytes[at + 1])
}

const fn nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => panic!("not a hex digit"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parses_alpha() {
        assert_eq!(
            css_hex("#4198d233"),
            Color32::from_rgba_unmultiplied(0x41, 0x98, 0xd2, 0x33)
        );
        assert_eq!(
            css_color("rgba(245, 196, 40, 0.2)"),
            Color32::from_rgba_unmultiplied(245, 196, 40, 51)
        );
        assert_eq!(css_color("rgba(0,0,0,1)"), Color32::BLACK);
        assert_eq!(css_hex("#292827"), Color32::from_rgb(0x29, 0x28, 0x27));
        assert_eq!(
            css_color("rgba(0, 0, 0, .9)"),
            Color32::from_rgba_unmultiplied(0, 0, 0, 230)
        );
        assert_eq!(
            css_color("rgb(86.6022727273, 167.1926136364, 211.6977272727)"),
            Color32::from_rgb(87, 167, 212)
        );
        assert_eq!(
            css_color("rgba(198.4615384615, 225.5192307692, 240.4615384615, 0.615)"),
            Color32::from_rgba_unmultiplied(198, 226, 240, 157)
        );
        assert_eq!(css_color("white"), Color32::WHITE);
    }
}
