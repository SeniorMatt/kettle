//! Interaction tests: click, hover and press keys in the real window layout (demo library, no network
//! needed) and check what happened. Positions are the layout numbers of `theme.rs` (top bar 34, nav row 44,
//! sidebar 273 wide, list rows 26 high).

use super::{AppUi, Tab};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use sr_client::Client;
use std::sync::{Arc, Once};

type State = (AppUi, Client, bool);

/// A demo library in a throw-away home directory; nothing is launched or sent anywhere.
fn harness() -> Harness<'static, State> {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("kettle-ui-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        // Safe here: set once, before any client is created, by the first test to run.
        unsafe {
            std::env::set_var("HOME", &dir);
            std::env::set_var("XDG_CONFIG_HOME", dir.join("config"));
            std::env::set_var("XDG_DATA_HOME", dir.join("data"));
            std::env::set_var("XDG_CACHE_HOME", dir.join("cache"));
            std::env::set_var("KETTLE_DEMO", "60");
            std::env::set_var(
                "KETTLE_DEMO_APPS",
                "440,570,730,105600,252490,292030,367520,413150,504230,620,1091500,1145360,1245620,374320",
            );
        }
    });
    let client = Client::new(Arc::new(|| {})).expect("client");
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).build_ui_state(
        |ui, state: &mut State| {
            if !state.2 {
                // The fonts take effect from the next frame: draw nothing in the first one.
                state.2 = true;
                AppUi::setup(ui.ctx());
                return;
            }
            state.0.show(ui, &mut state.1)
        },
        (AppUi::default(), client, false),
    );
    frames(&mut h, 4);
    h
}

fn frames(h: &mut Harness<'_, State>, n: usize) {
    for _ in 0..n {
        h.step();
    }
}

fn press(h: &mut Harness<'_, State>, pos: Pos2) {
    h.event(Event::PointerMoved(pos));
    frames(h, 1);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        frames(h, 1);
    }
    frames(h, 2);
}

fn key(h: &mut Harness<'_, State>, key: Key) {
    for pressed in [true, false] {
        h.event(Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: Modifiers::NONE });
        frames(h, 1);
    }
    frames(h, 2);
}

/// Centre of list row `i` (the list starts under the Home, drop-down and search rows).
fn row(i: usize) -> Pos2 {
    pos2(120.0, 203.0 + 26.0 * i as f32 + 13.0)
}

#[test]
fn clicking_a_list_row_selects_the_game() {
    let mut h = harness();
    assert!(h.state().1.state().selected.is_none());
    // The list starts with section headers; one of the first rows is a game.
    let mut hit = false;
    for i in 0..12 {
        // The left edge is a header's fold square and part of a game row.
        press(&mut h, pos2(14.0, row(i).y));
        if h.state().1.state().selected.is_some() {
            hit = true;
            break;
        }
    }
    assert!(hit, "no click on the first rows selected a game");
}

#[test]
fn collection_header_toggle_folds_only() {
    let mut h = harness();
    let before = h.state().0.sidebar.collapsed_count();
    // Row 0 is a header: its fold square sits at the left.
    press(&mut h, pos2(14.0, row(0).y));
    assert_eq!(h.state().0.sidebar.collapsed_count(), before + 1, "the toggle did not fold");
    assert!(h.state().1.state().collections.selected.is_none(), "the toggle must not open the collection");
    press(&mut h, pos2(14.0, row(0).y));
    assert_eq!(h.state().0.sidebar.collapsed_count(), before, "the toggle did not unfold");
}

#[test]
fn collection_header_click_opens_the_page_and_back_returns() {
    let mut h = harness();
    let before = h.state().0.sidebar.collapsed_count();
    // The title, and the empty space at the right, open the collection and do not fold it.
    for x in [70.0, 200.0] {
        press(&mut h, pos2(x, row(0).y));
        assert_eq!(h.state().0.sidebar.collapsed_count(), before, "a header click must not fold");
        assert!(h.state().1.state().collections.selected.is_some(), "a header click should open the collection");
        press(&mut h, pos2(23.0, 46.0));
        assert!(h.state().1.state().collections.selected.is_none(), "back should return to all games");
    }
}

#[test]
fn menu_bar_switches_menu_on_hover() {
    let mut h = harness();
    press(&mut h, pos2(50.0, 17.0));
    let first = h.state().0.bar.open;
    assert!(first.is_some(), "a click on the first title opens its menu");
    h.event(Event::PointerMoved(pos2(110.0, 17.0)));
    frames(&mut h, 3);
    h.event(Event::PointerMoved(pos2(170.0, 17.0)));
    frames(&mut h, 3);
    assert_ne!(h.state().0.bar.open, first, "moving over another title should switch the open menu");
}

#[test]
fn nav_tabs_and_back_arrow() {
    let mut h = harness();
    assert_eq!(h.state().0.tab, Tab::Library);
    // STORE is the first tab text (x 77...).
    press(&mut h, pos2(95.0, 46.0));
    assert_eq!(h.state().0.tab, Tab::Store);
    // The back arrow returns to the library.
    press(&mut h, pos2(23.0, 46.0));
    assert_eq!(h.state().0.tab, Tab::Library);
}

#[test]
fn escape_closes_only_the_open_popup() {
    let mut h = harness();
    press(&mut h, pos2(95.0, 46.0));
    press(&mut h, pos2(23.0, 46.0));
    press(&mut h, row(1));
    let selected = h.state().1.state().selected;
    // The filter button of the search row opens a popup.
    press(&mut h, pos2(249.0, 188.0));
    assert!(egui::Popup::is_any_open(&h.ctx), "the filter popup should be open");
    key(&mut h, Key::Escape);
    assert!(!egui::Popup::is_any_open(&h.ctx), "Escape should close the popup");
    assert_eq!(h.state().1.state().selected, selected, "Escape must not also go back");
    assert_eq!(h.state().0.tab, Tab::Library);
}

#[test]
fn collection_page_shows_the_games_of_the_clicked_section() {
    let mut h = harness();
    for id in ["favorite", "uc-demo", "all"] {
        if id == "all" {
            // Fold the sections above so the "all" header is within view.
            for above in ["favorite", "uc-demo"] {
                let i = h.state().0.sidebar.header_row(above).expect("header");
                press(&mut h, pos2(14.0, row(i).y));
            }
        }
        let i = h.state().0.sidebar.header_row(id).unwrap_or_else(|| panic!("no header {id}"));
        press(&mut h, pos2(120.0, row(i).y));
        let (app, client, _) = h.state();
        let mut want = app.sidebar.section_apps(client, id);
        let mut got = app.library.shown_apps(client);
        want.sort_unstable();
        got.sort_unstable();
        assert!(!want.is_empty(), "section {id} is empty");
        assert_eq!(got, want, "the page for {id} differs from its sidebar section");
        press(&mut h, pos2(23.0, 46.0));
    }
}

#[test]
fn game_page_asks_for_a_recommendation_after_enough_play_time() {
    let mut h = harness();
    for i in 0..12 {
        press(&mut h, pos2(14.0, row(i).y));
        if let Some(app) = h.state().1.state().selected {
            let g = h.state().1.state().game(app).cloned().expect("selected game");
            let due = g.playtime_minutes >= 120 && g.app_id.0 & 0x8000_0000 == 0;
            assert_eq!(
                h.query_by_label_contains("Would you recommend this game").is_some(),
                due,
                "{} ({} min)",
                g.name,
                g.playtime_minutes
            );
            return;
        }
    }
    panic!("no click selected a game");
}
