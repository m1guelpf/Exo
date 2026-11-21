//! The fonts message bodies are laid out with: Exo's fonts under the family names the body
//! stylesheets use, and the generic families resolved the way Chromium on macOS resolves them.

use std::sync::Arc;

use blitz_dom::FontContext;
use exo_fonts::{
    MONO_EMAIL, SANS_BOLD, SANS_BOLD_ITALIC, SANS_ITALIC, SANS_LIGHT, SANS_REGULAR, SANS_SEMIBOLD,
    SANS_SEMIBOLD_ITALIC,
};
use parley::fontique::{
    Blob, Collection, CollectionOptions, FallbackKey, FontInfoOverride, FontStyle, FontWeight,
    GenericFamily, Script, SourceCache,
};

/// The `@font-face` rules for message bodies, each drawn in one of Exo's fonts:
/// `(family, weight, italic, font)`.
///
/// Bodies use `Exo Sans Email`, which maps 700 to the true Bold; the quoting ellipsis uses the
/// UI's `Exo Sans`, whose 700 is the Semibold. A browser engine laying a body out (printing)
/// declares the same faces.
pub const EMAIL_FACES: &[(&str, u16, bool, &exo_fonts::Font)] = &[
    ("Exo Sans Email", 400, false, &SANS_REGULAR),
    ("Exo Sans Email", 400, true, &SANS_ITALIC),
    ("Exo Sans Email", 700, false, &SANS_BOLD),
    ("Exo Sans Email", 700, true, &SANS_BOLD_ITALIC),
    ("Exo Sans", 100, false, &SANS_LIGHT),
    ("Exo Sans", 400, false, &SANS_REGULAR),
    ("Exo Sans", 400, true, &SANS_ITALIC),
    ("Exo Sans", 700, false, &SANS_SEMIBOLD),
    ("Exo Sans", 700, true, &SANS_SEMIBOLD_ITALIC),
    ("Exo Mono Email", 400, false, &MONO_EMAIL),
];

/// Names senders use for the macOS system font, which Chromium resolves and Stylo doesn't.
const SYSTEM_FONT_ALIASES: &[&str] = &["-apple-system", "BlinkMacSystemFont"];
const SYSTEM_FONT: &str = "/System/Library/Fonts/SFNS.ttf";

/// The macOS family Chromium draws mathematical alphanumerics with.
const MATH_FONT: &str = "STIX Two Math";

/// Chromium's default font settings on macOS: which installed family each CSS generic means.
const MAC_GENERICS: &[(GenericFamily, &str)] = &[
    (GenericFamily::SansSerif, "Helvetica"),
    (GenericFamily::Serif, "Times"),
    (GenericFamily::Monospace, "Courier"),
    (GenericFamily::Cursive, "Apple Chancery"),
    (GenericFamily::Fantasy, "Papyrus"),
    (GenericFamily::SystemUi, "BlinkMacSystemFont"),
    (GenericFamily::UiSansSerif, "BlinkMacSystemFont"),
    (GenericFamily::Emoji, "Apple Color Emoji"),
];

/// A font collection shared by every message view: the email faces plus the system's fonts.
///
/// Building it scans the system fonts once; [`Fonts::context`] hands out cheap handles onto the
/// same collection.
#[derive(Clone)]
pub struct Fonts {
    context: FontContext,
}

impl Fonts {
    /// Registers the email faces alongside the system fonts.
    pub fn load() -> Self {
        let mut collection = Collection::new(CollectionOptions {
            shared: true,
            system_fonts: true,
        });
        for &(family, weight, italic, font) in EMAIL_FACES {
            collection.register_fonts(
                Blob::new(Arc::new(font.data)),
                Some(FontInfoOverride {
                    family_name: Some(family),
                    weight: Some(FontWeight::new(f32::from(weight))),
                    style: Some(if italic {
                        FontStyle::Italic
                    } else {
                        FontStyle::Normal
                    }),
                    ..FontInfoOverride::default()
                }),
            );
        }
        if let Ok(bytes) = std::fs::read(SYSTEM_FONT) {
            let blob = Blob::new(Arc::new(bytes));
            for &alias in SYSTEM_FONT_ALIASES {
                collection.register_fonts(
                    blob.clone(),
                    Some(FontInfoOverride {
                        family_name: Some(alias),
                        ..FontInfoOverride::default()
                    }),
                );
            }
        }
        for &(generic, name) in MAC_GENERICS {
            // Other platforms keep fontique's own choice for that generic.
            if let Some(id) = collection.family_id(name) {
                collection.set_generic_families(generic, std::iter::once(id));
            }
        }
        // Mathematical alphanumerics (`𝓇𝒶𝓌`) are common-script characters, which take the
        // script of the text around them; CoreText's cascade doesn't cover them, and Chromium
        // draws them with STIX Two Math (as exo-ui's fallback chain does). Last, so nothing the
        // cascade had moves.
        if let Some(math) = collection.family_id(MATH_FONT) {
            for script in [Script::COMMON, Script::from_bytes(*b"Latn")] {
                let key = FallbackKey::new(script, None);
                let mut families: Vec<_> = collection.fallback_families(key).collect();
                if !families.contains(&math) {
                    families.push(math);
                    collection.set_fallbacks(key, families.into_iter());
                }
            }
        }
        Self {
            context: FontContext {
                collection,
                source_cache: SourceCache::new_shared(),
            },
        }
    }

    /// A handle onto the shared collection, for one Blitz document.
    pub(crate) fn context(&self) -> FontContext {
        self.context.clone()
    }
}
