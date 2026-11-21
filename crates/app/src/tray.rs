//! The system tray icon. On Linux it is a StatusNotifierItem over D-Bus (KDE and most desktops, GNOME with the
//! AppIndicator extension), made with the `ksni` crate. It runs on its own thread: menu clicks become [`Msg`]s
//! that the window loop reads, and the UI never waits for D-Bus. Without a tray host (plain GNOME), `start`
//! never becomes active and closing the window just quits. Windows: not built yet (stub).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, OnceLock};

/// What the user asked for in the tray menu.
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    Show,
    Store,
    Library,
    Friends,
    Downloads,
    Settings,
    Exit,
}

pub struct Tray {
    rx: Receiver<Msg>,
    active: Arc<AtomicBool>,
    #[cfg(all(unix, not(target_os = "macos")))]
    handle: Arc<OnceLock<imp::Handle>>,
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    _unused: OnceLock<()>,
}

impl Tray {
    /// Start the tray in the background. `wake` repaints the window loop when a menu item is chosen.
    pub fn start(wake: Arc<dyn Fn() + Send + Sync>, icon_rgba: (u32, u32, Vec<u8>)) -> Tray {
        let (tx, rx) = channel();
        let active = Arc::new(AtomicBool::new(false));
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let handle = Arc::new(OnceLock::new());
            imp::spawn(tx, wake, icon_rgba, active.clone(), handle.clone());
            Tray { rx, active, handle }
        }
        #[cfg(not(all(unix, not(target_os = "macos"))))]
        {
            let _ = (tx, wake, icon_rgba);
            Tray { rx, active, _unused: OnceLock::new() }
        }
    }

    /// True once the tray icon is really shown (a tray host exists).
    pub fn active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    pub fn try_recv(&self) -> Option<Msg> {
        self.rx.try_recv().ok()
    }

    /// Remove the icon (called when the setting is turned off or the app quits).
    pub fn stop(&self) {
        #[cfg(all(unix, not(target_os = "macos")))]
        if let Some(h) = self.handle.get() {
            let _ = h.shutdown();
        }
        self.active.store(false, Ordering::Relaxed);
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use super::Msg;
    use ksni::blocking::TrayMethods;
    use ksni::menu::StandardItem;
    use ksni::{Icon, MenuItem};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::Sender;
    use std::sync::{Arc, OnceLock};

    pub type Handle = ksni::blocking::Handle<KettleTray>;

    pub struct KettleTray {
        tx: Sender<Msg>,
        wake: Arc<dyn Fn() + Send + Sync>,
        icon: Vec<Icon>,
    }

    impl KettleTray {
        fn send(&self, m: Msg) {
            let exit = m == Msg::Exit;
            let _ = self.tx.send(m);
            (self.wake)();
            if exit {
                // While the window is hidden to the tray, some desktops never let it draw again, so the window
                // loop may not see the message: quit from here if it hasn't after a moment.
                let _ = std::thread::Builder::new().name("tray-exit".into()).spawn(|| {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    std::process::exit(0);
                });
            }
        }
    }

    fn entry(label: &str, msg: Msg) -> MenuItem<KettleTray> {
        StandardItem {
            label: label.into(),
            activate: Box::new(move |t: &mut KettleTray| t.send(msg.clone())),
            ..Default::default()
        }
        .into()
    }

    impl ksni::Tray for KettleTray {
        fn id(&self) -> String {
            "kettle".into()
        }

        fn title(&self) -> String {
            "Kettle".into()
        }

        fn icon_pixmap(&self) -> Vec<Icon> {
            self.icon.clone()
        }

        /// Left click: show and raise the window.
        fn activate(&mut self, _x: i32, _y: i32) {
            self.send(Msg::Show);
        }

        fn menu(&self) -> Vec<MenuItem<Self>> {
            vec![
                entry("Store", Msg::Store),
                entry("Library", Msg::Library),
                entry("Friends", Msg::Friends),
                entry("Downloads", Msg::Downloads),
                entry("Settings", Msg::Settings),
                MenuItem::Separator,
                entry("Exit Kettle", Msg::Exit),
            ]
        }
    }

    /// RGBA pixels to the ARGB32 (network byte order) the tray protocol wants.
    fn to_argb(w: u32, h: u32, rgba: &[u8]) -> Vec<Icon> {
        let data = rgba.as_chunks::<4>().0.iter().flat_map(|&[r, g, b, a]| [a, r, g, b]).collect();
        vec![Icon { width: w as i32, height: h as i32, data }]
    }

    pub fn spawn(
        tx: Sender<Msg>,
        wake: Arc<dyn Fn() + Send + Sync>,
        icon: (u32, u32, Vec<u8>),
        active: Arc<AtomicBool>,
        out: Arc<OnceLock<Handle>>,
    ) {
        let _ = std::thread::Builder::new().name("tray-start".into()).spawn(move || {
            let tray = KettleTray { tx, wake, icon: to_argb(icon.0, icon.1, &icon.2) };
            // Fails (no tray host, no session bus) on desktops without a tray: then Kettle behaves as before.
            if let Ok(handle) = tray.spawn() {
                let _ = out.set(handle);
                active.store(true, Ordering::Relaxed);
            }
        });
    }
}
