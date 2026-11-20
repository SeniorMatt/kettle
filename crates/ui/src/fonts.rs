//! Bundled fonts: Figtree (closest free match to the original UI font),
//! Inter as fallback (Cyrillic, Greek), Phosphor for icons. All OFL/MIT.

use crate::theme::{FAMILY_BOLD, FAMILY_MEDIUM};
use egui::{FontData, FontDefinitions, FontFamily};
use std::sync::Arc;

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, bytes: &'static [u8]| {
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    };
    add("figtree", include_bytes!("../assets/fonts/Figtree-Regular.ttf"));
    add("figtree-medium", include_bytes!("../assets/fonts/Figtree-Medium.ttf"));
    add("figtree-bold", include_bytes!("../assets/fonts/Figtree-Bold.ttf"));
    add("inter", include_bytes!("../assets/fonts/Inter-Regular.ttf"));

    // Proportional: Figtree, then Inter, then egui's defaults (emoji etc.).
    let defaults = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let chain = |first: &str| {
        let mut v = vec![first.to_string(), "inter".to_string()];
        v.extend(defaults.iter().cloned());
        v
    };
    fonts.families.insert(FontFamily::Proportional, chain("figtree"));
    fonts.families.insert(FontFamily::Name(FAMILY_MEDIUM.into()), chain("figtree-medium"));
    fonts.families.insert(FontFamily::Name(FAMILY_BOLD.into()), chain("figtree-bold"));

    // Icons resolve in every family. Phosphor goes right after Figtree: Inter and
    // egui's emoji font have their own Private Use Area glyphs that would win,
    // and Phosphor can't go first because it maps some ASCII letters too.
    fonts
        .font_data
        .insert("phosphor".into(), Arc::new(FontData::from_static(egui_phosphor::Variant::Regular.font_bytes())));
    for fam in [FontFamily::Proportional, FontFamily::Name(FAMILY_MEDIUM.into()), FontFamily::Name(FAMILY_BOLD.into())]
    {
        if let Some(list) = fonts.families.get_mut(&fam) {
            list.insert(1, "phosphor".into());
        }
    }
    ctx.set_fonts(fonts);
}
