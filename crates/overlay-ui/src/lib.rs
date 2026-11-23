//! The in-game overlay's look (Shift+Tab), drawn with egui inside the game's own frames by the overlay hook
//! library. This crate only draws; it never talks to Steam or Kettle. The hook fills [`OverlayData`] from what
//! Kettle sends it and carries out the [`OverlayAction`]s.

/// Everything the overlay shows. Filled by the hook from Kettle's messages; all fields may be empty.
#[derive(Debug, Clone, Default)]
pub struct OverlayData {
    pub app_id: u32,
    pub game_name: String,
    /// Minutes played in this session and in total.
    pub session_minutes: u32,
    pub total_minutes: u32,
    /// Local time, already formatted ("21:34").
    pub clock: String,
    pub account_name: String,
    pub friends: Vec<OverlayFriend>,
    pub achievements: Option<OverlayAchievements>,
    pub perf: Option<OverlayPerf>,
}

#[derive(Debug, Clone, Default)]
pub struct OverlayFriend {
    pub steam_id: u64,
    pub name: String,
    /// "Online", "Away", or the game they are playing.
    pub status: String,
    pub in_game: bool,
    pub online: bool,
    /// Avatar as RGBA pixels (square), if loaded.
    pub avatar: Option<(usize, Vec<u8>)>,
}

#[derive(Debug, Clone, Default)]
pub struct OverlayAchievements {
    pub unlocked: u32,
    pub total: u32,
    /// Most recent first: (name, description, unlock time as unix seconds).
    pub recent: Vec<(String, String, u64)>,
}

#[derive(Debug, Clone, Default)]
pub struct OverlayPerf {
    pub fps: f32,
    pub frame_ms: f32,
}

/// What the user did in the overlay; the hook carries these out.
#[derive(Debug, Clone, PartialEq)]
pub enum OverlayAction {
    /// Close the overlay and give input back to the game.
    Close,
    OpenUrl(String),
    OpenChat(u64),
    TakeScreenshot,
    TogglePerfHud(bool),
}

/// The overlay's own view state (open panels, scroll, text fields). Kept by the hook between frames.
#[derive(Debug, Default)]
pub struct OverlayState {
    pub perf_hud: bool,
}

/// Draw the whole overlay for one frame (the dimmed backdrop, top and bottom bars, open panels).
pub fn show(ctx: &egui::Context, data: &OverlayData, state: &mut OverlayState) -> Vec<OverlayAction> {
    let mut actions = Vec::new();
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.label(&data.game_name);
        if ui.button("Close").clicked() {
            actions.push(OverlayAction::Close);
        }
    });
    let _ = state;
    actions
}

/// Draw only the small performance HUD (shown while the overlay is closed and the HUD is on).
pub fn show_hud(ctx: &egui::Context, perf: &OverlayPerf) {
    egui::Area::new(egui::Id::new("kettle-hud")).fixed_pos([8.0, 8.0]).show(ctx, |ui| {
        ui.label(format!("{:.0} FPS  {:.1} ms", perf.fps, perf.frame_ms));
    });
}
