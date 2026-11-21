//! Interface settings: animations, UI scale, start page, tray. Plain data, saved with the other settings.

use serde::{Deserialize, Serialize};

pub const MIN_SCALE_PERCENT: u32 = 90;
pub const MAX_SCALE_PERCENT: u32 = 150;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnimationMode {
    /// Every animation, including movement.
    #[default]
    Full,
    /// Fades only, nothing moves.
    Reduced,
    /// Everything changes at once.
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StartPage {
    #[default]
    Library,
    Store,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InterfaceSettings {
    pub animations: AnimationMode,
    /// UI scale in percent (90 to 150).
    pub scale_percent: u32,
    pub start_page: StartPage,
    /// Small game icons in the library sidebar.
    pub sidebar_icons: bool,
    pub smooth_scrolling: bool,
    /// Show Kettle in the system tray.
    pub tray: bool,
    /// Closing the window keeps Kettle running in the tray (only when the tray is on).
    pub close_to_tray: bool,
    /// Start with the window hidden in the tray.
    pub start_minimized: bool,
}

impl Default for InterfaceSettings {
    fn default() -> Self {
        InterfaceSettings {
            animations: AnimationMode::Full,
            scale_percent: 100,
            start_page: StartPage::Library,
            sidebar_icons: true,
            smooth_scrolling: true,
            tray: true,
            close_to_tray: true,
            start_minimized: false,
        }
    }
}

impl InterfaceSettings {
    pub fn clamped(mut self) -> Self {
        self.scale_percent = self.scale_percent.clamp(MIN_SCALE_PERCENT, MAX_SCALE_PERCENT);
        self
    }

    /// egui `zoom_factor` for the scale.
    pub fn zoom(&self) -> f32 {
        self.scale_percent.clamp(MIN_SCALE_PERCENT, MAX_SCALE_PERCENT) as f32 / 100.0
    }
}
