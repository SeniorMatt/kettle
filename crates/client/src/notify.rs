//! Desktop notifications. Local only: a message to the system's notification service over D-Bus (Linux);
//! nothing leaves the computer. On other systems this does nothing.

use super::Client;
use super::download_settings::NotifyKind;

impl Client {
    /// Show a notification when the user turned this kind on. Never blocks the UI (D-Bus runs on a thread).
    pub fn notify(&self, kind: NotifyKind, title: &str, body: &str) {
        let n = &self.state.settings.notifications;
        if n.wants(kind) {
            show(title, body, n.sound);
        }
    }
}

/// Send one notification from a short-lived thread. Errors (no notification service) are ignored: a missing
/// popup must never become an app error.
#[cfg(target_os = "linux")]
pub fn show(title: &str, body: &str, sound: bool) {
    let (title, body) = (title.to_string(), body.to_string());
    let _ = std::thread::Builder::new().name("notify".into()).spawn(move || {
        let mut n = notify_rust::Notification::new();
        n.appname("Kettle").summary(&title).body(&body).icon("kettle").timeout(6000);
        if sound {
            n.sound_name("message-new-instant");
        }
        let _ = n.show();
    });
}

#[cfg(not(target_os = "linux"))]
pub fn show(_title: &str, _body: &str, _sound: bool) {}
