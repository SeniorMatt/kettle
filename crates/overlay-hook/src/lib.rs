//! `libkettle_overlay.so`: Kettle's in-game overlay, drawn inside the game's own frames.
//!
//! Loaded with `LD_PRELOAD` (OpenGL, SDL and Xlib hooks) and as the Vulkan implicit layer
//! `VK_LAYER_KETTLE_overlay` (manifest in `packaging/`). Both are switched on by Kettle with
//! `KETTLE_OVERLAY=1` and find Kettle through `KETTLE_OVERLAY_SOCKET`. See ARCHITECTURE.md.
//!
//! * `state`: the one overlay of the process (egui context, data from Kettle, frame timing).
//! * `input`: the input state machine (pure, tested): Shift+Tab, what is swallowed while open.
//! * `sdl`, `xlib`: input hooks that feed it and release and restore the game's grabs and cursor.
//! * `gl`, `vk`: renderers that draw just before the game's swap or present.
//! * `real`: finding the real functions behind the hooks, and the `dlsym` hook.
//! * `ipc`: the socket to Kettle.
#![allow(clippy::missing_safety_doc, clippy::missing_transmute_annotations)]

pub mod gl;
pub mod input;
pub mod ipc;
pub mod real;
pub mod sdl;
pub mod state;
pub mod vk;
pub mod xlib;
