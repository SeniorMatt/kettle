//! Bundled fonts: Figtree (closest free match to the original UI font),
//! Inter as fallback (Cyrillic, Greek), Phosphor for icons. All OFL/MIT.
//! If the user has Motiva Sans installed as a system font, it goes first: we never
//! ship it (it is licensed) and never unpack it from the official client's files.

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

    // The user's own Motiva Sans, when installed: regular, medium, bold (each falls back to the next
    // weight down, then to Figtree).
    let motiva = system_motiva();
    for (name, bytes) in [("motiva", &motiva[0]), ("motiva-medium", &motiva[1]), ("motiva-bold", &motiva[2])] {
        if let Some(b) = bytes {
            fonts.font_data.insert(name.into(), Arc::new(FontData::from_owned(b.clone())));
        }
    }
    let pick = |want: &[&str]| want.iter().find(|n| fonts.font_data.contains_key(**n)).map(|n| n.to_string());
    let regular = pick(&["motiva"]);
    let medium = pick(&["motiva-medium", "motiva"]);
    let bold = pick(&["motiva-bold", "motiva-medium", "motiva"]);

    // Proportional: (Motiva Sans,) Figtree, then Inter, then egui's defaults (emoji etc.).
    let defaults = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let chain = |first: Option<String>, figtree: &str| {
        let mut v: Vec<String> = first.into_iter().collect();
        v.push(figtree.to_string());
        v.push("inter".to_string());
        v.extend(defaults.iter().cloned());
        v
    };
    fonts.families.insert(FontFamily::Proportional, chain(regular, "figtree"));
    fonts.families.insert(FontFamily::Name(FAMILY_MEDIUM.into()), chain(medium, "figtree-medium"));
    fonts.families.insert(FontFamily::Name(FAMILY_BOLD.into()), chain(bold, "figtree-bold"));

    // Icons resolve in every family. Phosphor goes right after Figtree: Inter and
    // egui's emoji font have their own Private Use Area glyphs that would win,
    // and Phosphor can't go first because it maps some ASCII letters too.
    fonts
        .font_data
        .insert("phosphor".into(), Arc::new(FontData::from_static(egui_phosphor::Variant::Regular.font_bytes())));
    for fam in [FontFamily::Proportional, FontFamily::Name(FAMILY_MEDIUM.into()), FontFamily::Name(FAMILY_BOLD.into())]
    {
        if let Some(list) = fonts.families.get_mut(&fam) {
            // Right after the first Latin font (Motiva Sans and Figtree both come before it).
            let at = list.iter().position(|n| n.starts_with("figtree")).map_or(1, |i| i + 1);
            list.insert(at, "phosphor".into());
        }
    }
    ctx.set_fonts(fonts);
}

/// Motiva Sans [regular, medium, bold] from the user's installed fonts, if any.
fn system_motiva() -> [Option<Vec<u8>>; 3] {
    let mut dirs =
        vec![std::path::PathBuf::from("/usr/share/fonts"), std::path::PathBuf::from("/usr/local/share/fonts")];
    if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
        dirs.push(home.join(".local/share/fonts"));
        dirs.push(home.join(".fonts"));
    }
    if let Some(data) = std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from) {
        dirs.push(data.join("fonts"));
    }
    let mut found: [Option<std::path::PathBuf>; 3] = [None, None, None];
    for d in dirs {
        scan(&d, 0, &mut found);
    }
    found.map(|p| p.and_then(|p| std::fs::read(p).ok()))
}

fn scan(dir: &std::path::Path, depth: u32, found: &mut [Option<std::path::PathBuf>; 3]) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let path = e.path();
        if path.is_dir() {
            if depth < 4 {
                scan(&path, depth + 1, found);
            }
            continue;
        }
        if let Some(slot) = motiva_slot(&e.file_name().to_string_lossy()) {
            found[slot].get_or_insert(path);
        }
    }
}

/// Which weight a font file name is: 0 regular, 1 medium, 2 bold. Only upright .ttf/.otf files.
fn motiva_slot(file: &str) -> Option<usize> {
    let f = file.to_ascii_lowercase().replace([' ', '_', '-'], "");
    let stem = f.strip_suffix(".ttf").or_else(|| f.strip_suffix(".otf"))?;
    match stem.strip_prefix("motivasans")? {
        "" | "regular" => Some(0),
        "medium" => Some(1),
        "bold" => Some(2),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::motiva_slot;

    #[test]
    fn motiva_file_names() {
        assert_eq!(motiva_slot("MotivaSans-Regular.ttf"), Some(0));
        assert_eq!(motiva_slot("Motiva Sans Medium.otf"), Some(1));
        assert_eq!(motiva_slot("MotivaSans-Bold.TTF"), Some(2));
        assert_eq!(motiva_slot("MotivaSans-BoldItalic.ttf"), None);
        assert_eq!(motiva_slot("MotivaSans-Regular.woff2"), None);
        assert_eq!(motiva_slot("Figtree-Regular.ttf"), None);
    }
}
