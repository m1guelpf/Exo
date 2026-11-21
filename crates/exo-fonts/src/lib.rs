#![deny(
    clippy::correctness,
    clippy::suspicious,
    clippy::complexity,
    clippy::perf
)]
//! Exo's fonts, compiled into the binary so nothing has to sit beside it.
//!
//! Each is an OFL font (Public Sans, Fira Mono, Inconsolata), instanced, sized, tracked and given
//! its own line metrics, in `assets/fonts` with their licence (`OFL.txt`) and provenance
//! (`SOURCES.txt`). Which CSS family and weight each one serves is up to exo-ui (UI text) and
//! exo-mail (message bodies).

/// A font file.
#[derive(Debug)]
pub struct Font {
    /// Its PostScript name, which is also its file stem.
    pub name: &'static str,
    /// The TrueType file.
    pub data: &'static [u8],
}

macro_rules! fonts {
    ($($(#[$doc:meta])* $id:ident = $name:literal;)*) => {
        $(
            $(#[$doc])*
            pub static $id: Font = Font {
                name: $name,
                data: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/", $name, ".ttf")),
            };
        )*
    };
}

fonts! {
    /// Exo Sans Light.
    SANS_LIGHT = "ExoSans-Light";
    /// Exo Sans.
    SANS_REGULAR = "ExoSans-Regular";
    /// Exo Sans Italic.
    SANS_ITALIC = "ExoSans-Italic";
    /// Exo Sans Semibold.
    SANS_SEMIBOLD = "ExoSans-Semibold";
    /// Exo Sans Semibold Italic.
    SANS_SEMIBOLD_ITALIC = "ExoSans-SemiboldItalic";
    /// Exo Sans Bold.
    SANS_BOLD = "ExoSans-Bold";
    /// Exo Sans Bold Italic.
    SANS_BOLD_ITALIC = "ExoSans-BoldItalic";
    /// Exo Sans Alt Light.
    SANS_ALT_LIGHT = "ExoSansAlt-Light";
    /// Exo Sans Alt.
    SANS_ALT_REGULAR = "ExoSansAlt-Regular";
    /// Exo Sans Alt Italic.
    SANS_ALT_ITALIC = "ExoSansAlt-Italic";
    /// Exo Sans Alt Semibold.
    SANS_ALT_SEMIBOLD = "ExoSansAlt-Semibold";
    /// Exo Sans Alt Semibold Italic.
    SANS_ALT_SEMIBOLD_ITALIC = "ExoSansAlt-SemiboldItalic";
    /// Exo Mono.
    MONO = "ExoMono-Regular";
    /// Exo Mono Email.
    MONO_EMAIL = "ExoMonoEmail-Regular";
}
