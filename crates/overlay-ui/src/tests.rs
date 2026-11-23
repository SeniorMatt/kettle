use super::*;
use egui::{Event, Key, Modifiers, PointerButton, pos2, vec2};
use egui_kittest::Harness;
use std::collections::HashMap;

fn avatar(seed: u8) -> Option<(usize, Vec<u8>)> {
    let n = 32;
    let mut px = Vec::with_capacity(n * n * 4);
    for y in 0..n {
        for x in 0..n {
            let on = ((x / 8 + y / 8) % 2) as u8;
            px.extend([60 + seed.wrapping_mul(37) / 2 + on * 80, 90 + seed.wrapping_mul(71) / 3, 120 + on * 40, 255]);
        }
    }
    Some((n, px))
}

fn sample() -> OverlayData {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let names = [
        "Abacus Avenger",
        "Barbarijan",
        "bendotcom",
        "boyd",
        "DanielJ",
        "JustJeff",
        "Keith",
        "Shinyklaxx",
        "smeehrr",
        "Steve",
    ];
    let mut friends = Vec::new();
    for (i, n) in names.iter().enumerate() {
        friends.push(OverlayFriend {
            steam_id: 1000 + i as u64,
            name: n.to_string(),
            status: if i < 2 { "Sample Game".into() } else { "Online".into() },
            in_game: i < 2,
            online: true,
            avatar: if i % 3 == 0 { None } else { avatar(i as u8) },
        });
    }
    OverlayData {
        app_id: 620,
        game_name: "SAMPLE GAME".into(),
        session_minutes: 80,
        total_minutes: 2304,
        clock: "1:04 PM".into(),
        date: "Thu, June 8, 2023".into(),
        account_name: "Lawrence".into(),
        account_avatar: None,
        friends,
        achievements: Some(OverlayAchievements {
            unlocked: 9,
            total: 63,
            recent: vec![
                ("Honour".into(), "Defeat the Dung Defender".into(), now - 600),
                ("Release".into(), "Defeat the Broken Vessel".into(), now - 4000),
                ("Beast".into(), "Destroy Herrah the Beast".into(), now - 90000),
                ("Grubfriend".into(), "Rescue half of the imprisoned grubs".into(), now - 200000),
                ("Respect".into(), "Defeat the Mantis Lords".into(), now - 500000),
            ],
        }),
        perf: Some(OverlayPerf { fps: 144.0, frame_ms: 6.9 }),
    }
}

type St = (OverlayData, OverlayState, Vec<OverlayAction>, bool);

fn harness(w: f32, h: f32) -> Harness<'static, St> {
    let mut hn = Harness::builder().with_size(vec2(w, h)).build_ui_state(
        |ui, s: &mut St| {
            let acts = show(ui.ctx(), &s.0, &mut s.1);
            s.2.extend(acts);
        },
        (sample(), OverlayState::default(), Vec::new(), false),
    );
    for _ in 0..4 {
        hn.step();
    }
    hn
}

fn click(h: &mut Harness<'_, St>, pos: egui::Pos2) {
    h.event(Event::PointerMoved(pos));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.step();
}

/// Screen position of a toolbar button (index 0..9) at the given window size.
fn tool_pos(w: f32, h: f32, i: usize) -> egui::Pos2 {
    let k = (h / 1125.0).max(0.75).min(w / 1800.0);
    let ox = (w - 2000.0 * k) / 2.0;
    let x0 = 1000.0 - ((9.0 - 1.0) * 43.0 + 36.0) / 2.0;
    pos2(ox + (x0 + i as f32 * 43.0 + 18.0) * k, h - 77.0 * k + 18.0 * k)
}

#[test]
fn opens_default_panels_and_escape_closes() {
    let mut h = harness(1600.0, 900.0);
    assert!(
        h.state().1.is_open(Panel::Friends)
            && h.state().1.is_open(Panel::Overview)
            && h.state().1.is_open(Panel::Achievements)
    );
    assert!(h.state().2.is_empty());
    h.event(Event::Key {
        key: Key::Escape,
        physical_key: Some(Key::Escape),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    assert!(h.state().2.contains(&OverlayAction::Close));
}

#[test]
fn back_to_game_button_closes() {
    let mut h = harness(1600.0, 900.0);
    let k = 900.0 / 1125.0;
    let ox = (1600.0 - 2000.0 * k) / 2.0;
    click(&mut h, pos2(ox + 1730.0 * k, 37.0 * k));
    assert!(h.state().2.contains(&OverlayAction::Close));
}

#[test]
fn toolbar_toggles_panels_and_fires_actions() {
    let mut h = harness(1600.0, 900.0);
    click(&mut h, tool_pos(1600.0, 900.0, 2)); // notes
    assert!(h.state().1.is_open(Panel::Notes));
    click(&mut h, tool_pos(1600.0, 900.0, 2));
    assert!(!h.state().1.is_open(Panel::Notes));
    click(&mut h, tool_pos(1600.0, 900.0, 4)); // screenshot
    assert!(h.state().2.contains(&OverlayAction::TakeScreenshot));
    click(&mut h, tool_pos(1600.0, 900.0, 8)); // performance monitor
    assert!(h.state().2.contains(&OverlayAction::TogglePerfHud(true)));
    assert!(h.state().1.perf_hud);
}

#[test]
fn friend_click_opens_chat_and_link_opens_url() {
    let mut h = harness(1600.0, 900.0);
    let k = 900.0 / 1125.0;
    let ox = (1600.0 - 2000.0 * k) / 2.0;
    // First row of the Friends panel: panel at (1551,163), list starts 83 units below, header 27.
    click(&mut h, pos2(ox + 1650.0 * k, (163.0 + 83.0 + 27.0 + 15.0) * k));
    assert!(h.state().2.contains(&OverlayAction::OpenChat(1000)), "{:?}", h.state().2);
    // Overview "Browse community guides" link.
    let acts_before = h.state().2.len();
    click(
        &mut h,
        pos2(ox + 260.0 * k, (168.0 + 58.0 + 22.0 + 52.0 + 22.0 + 42.0 + 22.0 + 80.0 + 22.0 + 70.0 + 22.0 + 22.0) * k),
    );
    assert!(
        h.state().2[acts_before..]
            .iter()
            .any(|a| matches!(a, OverlayAction::OpenUrl(u) if u.contains("/app/620/guides"))),
        "{:?}",
        h.state().2
    );
}

#[test]
fn works_from_720p_to_4k() {
    for (w, hh) in [(1280.0, 720.0), (1920.0, 1080.0), (2560.0, 1440.0), (3840.0, 2160.0)] {
        let mut h = harness(w, hh);
        for i in [0usize, 1, 2, 3, 5, 6, 7] {
            click(&mut h, tool_pos(w, hh, i));
        }
        for _ in 0..3 {
            h.step();
        }
    }
}

#[test]
fn hud_draws() {
    let mut h = Harness::builder()
        .with_size(vec2(800.0, 600.0))
        .build_ui_state(|ui, _: &mut ()| show_hud(ui.ctx(), &OverlayPerf { fps: 60.0, frame_ms: 16.6 }), ());
    for _ in 0..4 {
        h.step();
    }
}

// ---- software renderer, for the snapshot PNGs --------------------------------------------------------------

struct Raster {
    w: usize,
    h: usize,
    px: Vec<[f32; 4]>, // premultiplied, 0..1
    tex: HashMap<egui::TextureId, egui::ColorImage>,
}

impl Raster {
    fn apply(&mut self, d: &egui::TexturesDelta) {
        for (id, deltas) in &d.set {
            for delta in deltas {
                let egui::ImageData::Color(img) = &delta.image;
                match delta.pos {
                    None => {
                        self.tex.insert(*id, (**img).clone());
                    }
                    Some([x, y]) => {
                        let t = self.tex.get_mut(id).expect("patch of unknown texture");
                        for j in 0..img.size[1] {
                            for i in 0..img.size[0] {
                                t.pixels[(y + j) * t.size[0] + x + i] = img.pixels[j * img.size[0] + i];
                            }
                        }
                    }
                }
            }
        }
    }

    fn sample(&self, id: egui::TextureId, u: f32, v: f32) -> [f32; 4] {
        let t = &self.tex[&id];
        let (tw, th) = (t.size[0] as f32, t.size[1] as f32);
        let x = (u * tw - 0.5).clamp(0.0, tw - 1.0);
        let y = (v * th - 0.5).clamp(0.0, th - 1.0);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(t.size[0] - 1), (y0 + 1).min(t.size[1] - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let g = |x: usize, y: usize| {
            let c = t.pixels[y * t.size[0] + x];
            [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0, c.a() as f32 / 255.0]
        };
        let (a, b, c, d) = (g(x0, y0), g(x1, y0), g(x0, y1), g(x1, y1));
        let mut o = [0.0; 4];
        for i in 0..4 {
            o[i] = (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy;
        }
        o
    }

    fn draw(&mut self, prims: &[egui::ClippedPrimitive]) {
        for cp in prims {
            let egui::epaint::Primitive::Mesh(m) = &cp.primitive else { continue };
            let clip = cp.clip_rect;
            for tri in m.indices.chunks(3) {
                let v = [m.vertices[tri[0] as usize], m.vertices[tri[1] as usize], m.vertices[tri[2] as usize]];
                let (minx, maxx) = (
                    v.iter().map(|a| a.pos.x).fold(f32::MAX, f32::min),
                    v.iter().map(|a| a.pos.x).fold(f32::MIN, f32::max),
                );
                let (miny, maxy) = (
                    v.iter().map(|a| a.pos.y).fold(f32::MAX, f32::min),
                    v.iter().map(|a| a.pos.y).fold(f32::MIN, f32::max),
                );
                let x0 = minx.max(clip.min.x).floor().max(0.0) as usize;
                let x1 = (maxx.min(clip.max.x).ceil() as usize).min(self.w);
                let y0 = miny.max(clip.min.y).floor().max(0.0) as usize;
                let y1 = (maxy.min(clip.max.y).ceil() as usize).min(self.h);
                let area = (v[1].pos.x - v[0].pos.x) * (v[2].pos.y - v[0].pos.y)
                    - (v[2].pos.x - v[0].pos.x) * (v[1].pos.y - v[0].pos.y);
                if area.abs() < 1e-6 {
                    continue;
                }
                for y in y0..y1 {
                    for x in x0..x1 {
                        let (px, py) = (x as f32 + 0.5001, y as f32 + 0.50007);
                        let w0 = ((v[1].pos.x - px) * (v[2].pos.y - py) - (v[2].pos.x - px) * (v[1].pos.y - py)) / area;
                        let w1 = ((v[2].pos.x - px) * (v[0].pos.y - py) - (v[0].pos.x - px) * (v[2].pos.y - py)) / area;
                        let w2 = 1.0 - w0 - w1;
                        if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                            continue;
                        }
                        let u = w0 * v[0].uv.x + w1 * v[1].uv.x + w2 * v[2].uv.x;
                        let vv = w0 * v[0].uv.y + w1 * v[1].uv.y + w2 * v[2].uv.y;
                        let mut c = [0.0f32; 4];
                        for i in 0..4 {
                            let f = |col: egui::Color32| [col.r(), col.g(), col.b(), col.a()][i] as f32 / 255.0;
                            c[i] = w0 * f(v[0].color) + w1 * f(v[1].color) + w2 * f(v[2].color);
                        }
                        let t = self.sample(m.texture_id, u, vv);
                        let s = [c[0] * t[0], c[1] * t[1], c[2] * t[2], c[3] * t[3]];
                        let d = &mut self.px[y * self.w + x];
                        for i in 0..4 {
                            d[i] = s[i] + d[i] * (1.0 - s[3]);
                        }
                    }
                }
            }
        }
    }

    fn write(&self, path: &std::path::Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut bytes = Vec::with_capacity(self.w * self.h * 3);
        for p in &self.px {
            for c in &p[..3] {
                bytes.push((c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
            }
        }
        let f = std::fs::File::create(path).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), self.w as u32, self.h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&bytes).unwrap();
    }
}

/// A made-up "game" picture: sky, sun, hills, a few boxes as a HUD. Not any game's art.
fn game_picture(w: usize, h: usize) -> Vec<[f32; 4]> {
    let mut px = vec![[0.0; 4]; w * h];
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let mut c = [0.25 + 0.3 * (1.0 - fy), 0.5 + 0.25 * (1.0 - fy), 0.85, 1.0];
            let sun = ((fx - 0.72).powi(2) * (w as f32 / h as f32).powi(2) + (fy - 0.25).powi(2)).sqrt();
            if sun < 0.09 {
                c = [1.0, 0.9, 0.55, 1.0];
            }
            let hill = 0.68 + 0.06 * (fx * 9.0).sin() + 0.04 * (fx * 23.0).cos();
            if fy > hill {
                c = [0.18 + 0.1 * fy, 0.45 - 0.1 * fy, 0.2, 1.0];
            }
            if fx < 0.2 && fy > 0.88 && fy < 0.97 && fx > 0.03 {
                c = [0.1, 0.1, 0.12, 1.0];
            }
            px[y * w + x] = c;
        }
    }
    px
}

fn render(w: usize, h: usize, state: &mut OverlayState, data: &OverlayData, frames: usize, hud_only: bool) -> Raster {
    let ctx = egui::Context::default();
    let mut r = Raster { w, h, px: game_picture(w, h), tex: HashMap::new() };
    install_fonts(&ctx);
    for i in 0..frames {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(w as f32, h as f32))),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx();
            if hud_only {
                show_hud(ctx, data.perf.as_ref().unwrap());
            } else {
                show(ctx, data, state);
            }
        });
        r.apply(&out.textures_delta);
        out.textures_delta.clear();
        if i + 1 == frames {
            let prims = ctx.tessellate(out.shapes, 1.0);
            r.draw(&prims);
        }
    }
    r
}

fn shots_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/overlay-shots")
}

#[test]
fn snapshot_png() {
    let data = sample();
    let mut st = OverlayState::default();
    let r = render(1600, 900, &mut st, &data, 4, false);
    r.write(&shots_dir().join("overlay-1600x900.png"));
    // Something was drawn over the picture: the middle of the sky is much darker than the picture's.
    let sky = r.px[100 * 1600 + 1300];
    assert!(sky[2] < 0.3, "backdrop should dim the game, got {sky:?}");

    let mut st = OverlayState { perf_hud: true, ..Default::default() };
    st.open[Panel::Notes as usize] = true;
    st.initialised = true;
    st.notes = "To do:\n- kill the bug bosses\n- find more notches".into();
    render(1600, 900, &mut st, &data, 4, false).write(&shots_dir().join("overlay-notes-1600x900.png"));

    render(1600, 900, &mut OverlayState::default(), &data, 4, true).write(&shots_dir().join("hud-1600x900.png"));
    render(3840, 2160, &mut OverlayState::default(), &data, 4, false).write(&shots_dir().join("overlay-3840x2160.png"));
    render(1280, 720, &mut OverlayState::default(), &data, 4, false).write(&shots_dir().join("overlay-1280x720.png"));
}
