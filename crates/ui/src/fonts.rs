//! Bundled fonts: Figtree (closest free match to the original UI font),
//! Inter as fallback (Cyrillic, Greek), Phosphor for icons. All OFL/MIT.
//! The desktop's own UI font goes first (KDE's `kdeglobals`, GNOME's `font-name`, else fontconfig's
//! `system-ui`), so Kettle matches the rest of the desktop; `KETTLE_FONT=builtin` keeps Figtree.
//! Without a desktop font, an installed Motiva Sans is used: we never ship it (it is licensed) and never
//! unpack it from the official client's files.

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
    let builtin = std::env::var("KETTLE_FONT").is_ok_and(|v| v == "builtin");
    let desktop = if builtin { [None, None, None] } else { desktop_font() };
    let motiva = if builtin || desktop[0].is_some() { [None, None, None] } else { system_motiva() };
    let own = if desktop[0].is_some() { desktop } else { motiva };
    for (name, bytes) in [("motiva", &own[0]), ("motiva-medium", &own[1]), ("motiva-bold", &own[2])] {
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

/// The desktop's UI font family name: KDE (`kdeglobals` `font=`), GNOME-like (`gsettings` `font-name`),
/// else fontconfig's `system-ui` alias.
fn desktop_family() -> String {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    if desktop.contains("kde") {
        let cfg = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")));
        if let Some(text) = cfg.and_then(|c| std::fs::read_to_string(c.join("kdeglobals")).ok())
            && let Some(family) = kde_font(&text)
        {
            return family;
        }
        return "Noto Sans".into(); // Plasma's default when the user never changed it
    }
    let out =
        std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "font-name"]).output();
    if let Ok(out) = out
        && out.status.success()
        && let Some(family) = gnome_font(&String::from_utf8_lossy(&out.stdout))
    {
        return family;
    }
    "system-ui".into()
}

/// `font=Noto Sans,10,-1,5,...` in the `[General]` group of `kdeglobals`.
fn kde_font(text: &str) -> Option<String> {
    let mut general = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            general = line == "[General]";
        } else if general && let Some(v) = line.strip_prefix("font=") {
            return v.split(',').next().map(str::trim).filter(|f| !f.is_empty()).map(String::from);
        }
    }
    None
}

/// `'Cantarell 11'` / `'Adwaita Sans 11'` → the family without the size.
fn gnome_font(value: &str) -> Option<String> {
    let v = value.trim().trim_matches('\'');
    let family = match v.rsplit_once(' ') {
        Some((f, size)) if size.parse::<f32>().is_ok() => f,
        _ => v,
    };
    Some(family.trim().to_string()).filter(|f| !f.is_empty())
}

/// [regular, medium, bold] files of the desktop font, found through fontconfig (`fc-match`).
fn desktop_font() -> [Option<Vec<u8>>; 3] {
    let family = desktop_family();
    let file = |weight: u32| {
        let out = std::process::Command::new("fc-match")
            .args(["-f", "%{file}", &format!("{family}:weight={weight}")])
            .output()
            .ok()?;
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        // egui reads TrueType/OpenType outlines only.
        let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
        if !out.status.success() || !(ext == "ttf" || ext == "otf") {
            return None;
        }
        std::fs::read(path).ok()
    };
    // fontconfig weights: 80 regular, 100 medium, 200 bold.
    [file(80), file(100), file(200)]
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
    fn desktop_font_names() {
        use super::{gnome_font, kde_font};
        assert_eq!(gnome_font("'Cantarell 11'\n").as_deref(), Some("Cantarell"));
        assert_eq!(gnome_font("'Adwaita Sans 11'").as_deref(), Some("Adwaita Sans"));
        let kde = "[KDE]\nfont=Wrong,1\n[General]\nColorScheme=BreezeDark\nfont=Noto Sans,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\n";
        assert_eq!(kde_font(kde).as_deref(), Some("Noto Sans"));
        assert_eq!(kde_font("[General]\n"), None);
    }

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
