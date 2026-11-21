//! The "Performance overlay" settings (MangoHud). The data types and the config text live in
//! `sr_steam::mangohud`; this file saves the setting and tells the UI whether MangoHud is installed.

pub use sr_steam::mangohud::{
    DEFAULT_TOGGLE_KEY, Detection, FPS_LIMIT_CHOICES, FontSize, HudSettings, INSTALL_HINTS, INSTALL_URL, Items,
    Position, Preset, TextColor,
};

use crate::Client;
use sr_steam::store;

impl Client {
    pub(crate) fn set_hud(&mut self, hud: HudSettings) {
        self.state.settings.hud = hud.sanitized();
        if let Err(e) = store::save_json(&crate::settings::Settings::path(), &self.state.settings) {
            self.state.notice = Some(format!("Couldn't save the settings: {e:#}"));
        }
    }
}

/// Is MangoHud installed? Looks at the disk, so call it when a page opens, not every frame.
pub fn detect() -> Detection {
    sr_steam::mangohud::detect()
}
