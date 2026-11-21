//! Kettle's own `steamclient.so`.
//!
//! Games that use the Steamworks API load a library called `steamclient` (directly through `libsteam_api.so`, or
//! through Proton's `lsteamclient` bridge) and talk to it through C++ interface objects. This crate builds that
//! library. It holds no Steam connection itself: every question that needs the account (who is signed in, what
//! is owned, tickets, stats) goes over a Unix socket to the running Kettle (see [`proto`]), which answers from its
//! own CM session. See `docs/steamclient-design.md`.

pub mod ipc;
pub mod proto;

#[cfg(feature = "exports")]
mod exports;
#[cfg(feature = "exports")]
mod iface;
#[cfg(feature = "exports")]
mod state;
