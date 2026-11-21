//! "Run games in Gamescope": the global setting, the per-game override and what the UI may ask about the
//! installed program. The command itself is built in `sr_steam::gamescope`.

pub use sr_steam::gamescope::{
    Detection, GamescopeGames, GamescopeSettings, INSTALL_HINTS, INSTALL_URL, MAX_SHARPNESS, Override, RESOLUTIONS,
    Upscaler, WindowMode, detect,
};

use crate::Client;
use sr_model::AppId;

impl Client {
    pub(crate) fn set_gamescope(&mut self, g: GamescopeSettings) {
        self.state.settings.gamescope = g.sanitized();
        self.save_settings();
    }

    pub(crate) fn set_game_gamescope(&mut self, app: AppId, o: Override) {
        let mut games = self.state.gamescope_games.clone();
        games.set(app.0, o);
        match games.save() {
            Ok(()) => self.state.gamescope_games = games,
            Err(e) => self.state.notice = Some(format!("Couldn't save the Gamescope choice: {e:#}")),
        }
    }

    /// Does `app` start inside Gamescope (the global switch and the game's own override)?
    pub fn gamescope_for(&self, app: AppId) -> bool {
        self.state.gamescope_games.get(app.0).resolve(self.state.settings.gamescope.enabled)
    }
}

/// The X display Gamescope made for a running game (`:N`), once its launcher has reported it.
pub fn gamescope_display(app: AppId) -> Option<String> {
    sr_steam::gamescope::read_display(app.0)
}
