//! Profile pages: anyone's public profile and editing your own.
//!
//! Reads come from the public community pages (see `sr_steam::community`). Editing needs a web session: for
//! every load or save a fresh access token is asked from Steam over the shared connection, turned into cookies
//! in memory, used for one request and dropped. Neither tokens nor cookies are stored in `State`, logged or
//! shown; errors are scrubbed before they reach the UI.

use crate::cm_hub::TokenCopy;
use crate::{Client, Event};
use sr_model::SteamId;
use sr_steam::community::{self, EditableProfile, ProfileXml, WebSession};
use sr_steam::{cm, redact};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// Profiles kept in memory.
const KEPT: usize = 24;
/// A cached profile older than this is fetched again when it is opened.
const FRESH: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq)]
pub enum ProfileLoad {
    Loading,
    Ready,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct ProfileEntry {
    pub load: ProfileLoad,
    pub xml: Option<ProfileXml>,
    pub level: Option<u32>,
    fetched: Instant,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum EditPhase {
    #[default]
    Idle,
    /// Asking Steam for a web session and the current values.
    Loading,
    /// `ProfileState::edit_original` holds the values the form started from.
    Editing,
    Saving,
    Failed(String),
}

#[derive(Debug, Clone, Default)]
pub struct ProfileState {
    /// The profile shown on the profile page (`None` = our own).
    pub viewing: Option<SteamId>,
    /// Bumped by `Command::ViewProfile`; the UI switches to the profile page when it changes.
    pub nav_seq: u64,
    pub profiles: HashMap<u64, ProfileEntry>,
    order: VecDeque<u64>,
    pub edit: EditPhase,
    /// The values the edit form started from (location and custom URL are kept as they were).
    pub edit_original: Option<EditableProfile>,
    /// Bumped when an edit has been saved.
    pub saved_seq: u64,
}

impl ProfileState {
    pub fn entry(&self, id: SteamId) -> Option<&ProfileEntry> {
        self.profiles.get(&id.0)
    }
}

#[derive(Debug)]
pub(crate) enum ProfileEvent {
    Loaded { id: u64, result: Result<(ProfileXml, Option<u32>), String> },
    EditLoaded(Result<EditableProfile, String>),
    Saved(Result<(), String>),
}

impl Client {
    fn own_id(&self) -> u64 {
        self.state.account.as_ref().map_or(0, |a| a.steam_id.0)
    }

    /// Show a profile: the page switches to it and it loads (or is taken from memory).
    pub(crate) fn view_profile(&mut self, id: SteamId) {
        let own = self.own_id();
        let p = &mut self.state.profile;
        p.viewing = (id.0 != own).then_some(id);
        p.nav_seq += 1;
        if id.0 != own {
            p.edit = EditPhase::Idle;
        }
        self.load_profile(id, false);
    }

    /// Fetch a profile unless a fresh copy is in memory.
    pub(crate) fn load_profile(&mut self, id: SteamId, force: bool) {
        if let Some(e) = self.state.profile.profiles.get(&id.0)
            && !force
            && (e.load == ProfileLoad::Loading || (e.load == ProfileLoad::Ready && e.fetched.elapsed() < FRESH))
        {
            return;
        }
        let demo = self.state.account.as_ref().is_some_and(|a| a.steam_id.0 == 0);
        let old = self.state.profile.profiles.get(&id.0).map(|e| (e.xml.clone(), e.level));
        let (xml, level) = old.unwrap_or((None, None));
        self.put_profile(id.0, ProfileEntry { load: ProfileLoad::Loading, xml, level, fetched: Instant::now() });
        if demo {
            let result = Ok((self.demo_profile(id), Some(42)));
            self.apply_profile(ProfileEvent::Loaded { id: id.0, result });
            return;
        }
        let generation = self.generation.load(Ordering::Relaxed);
        self.spawn_profile(generation, id.0, move || {
            let xml = community::fetch_profile_xml(id.0).map_err(|e| format!("{e:#}"))?;
            // The level is a nice extra: its failure doesn't fail the page.
            let level = community::fetch_level(id.0).ok().flatten();
            Ok((xml, level))
        });
    }

    fn spawn_profile(
        &self,
        generation: u64,
        id: u64,
        job: impl FnOnce() -> Result<(ProfileXml, Option<u32>), String> + Send + 'static,
    ) {
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        self.rt.spawn_blocking(move || {
            let result = job();
            let _ = tx.blocking_send(Event::Profile { generation, ev: ProfileEvent::Loaded { id, result } });
            wake();
        });
    }

    fn put_profile(&mut self, id: u64, entry: ProfileEntry) {
        let p = &mut self.state.profile;
        p.profiles.insert(id, entry);
        p.order.retain(|x| *x != id);
        p.order.push_back(id);
        while p.order.len() > KEPT {
            if let Some(old) = p.order.pop_front() {
                p.profiles.remove(&old);
            }
        }
    }

    pub(crate) fn apply_profile(&mut self, ev: ProfileEvent) {
        match ev {
            ProfileEvent::Loaded { id, result } => match result {
                Ok((xml, level)) => {
                    self.put_profile(
                        id,
                        ProfileEntry { load: ProfileLoad::Ready, xml: Some(xml), level, fetched: Instant::now() },
                    );
                }
                Err(e) => {
                    if let Some(entry) = self.state.profile.profiles.get_mut(&id) {
                        entry.load = ProfileLoad::Failed(e);
                    }
                }
            },
            ProfileEvent::EditLoaded(result) => match result {
                Ok(p) => {
                    self.state.profile.edit_original = Some(p);
                    self.state.profile.edit = EditPhase::Editing;
                }
                Err(e) => self.state.profile.edit = EditPhase::Failed(e),
            },
            ProfileEvent::Saved(result) => match result {
                Ok(()) => {
                    let me = self.own_id();
                    let p = &mut self.state.profile;
                    p.edit = EditPhase::Idle;
                    p.edit_original = None;
                    p.saved_seq += 1;
                    self.state.notice = Some("Profile saved".into());
                    // Show the new values, and ask Steam for our persona again (the header name).
                    self.load_profile(SteamId(me), true);
                    self.friends_request_own();
                }
                Err(e) => self.state.profile.edit = EditPhase::Failed(e),
            },
        }
    }

    /// Start editing our profile: get a web session and the current values.
    pub(crate) fn profile_edit_start(&mut self) {
        if self.state.account.as_ref().is_some_and(|a| a.steam_id.0 == 0) {
            // Demo account: a form with made-up values, nothing is sent anywhere.
            self.state.profile.edit_original = Some(EditableProfile {
                persona_name: "demo".into(),
                real_name: "Demo Person".into(),
                summary: "A demo profile.".into(),
                ..Default::default()
            });
            self.state.profile.edit = EditPhase::Editing;
            return;
        }
        self.state.profile.edit = EditPhase::Loading;
        self.web_task(None);
    }

    /// Save the edited values.
    pub(crate) fn profile_edit_save(&mut self, draft: EditableProfile) {
        if let Err(e) = community::validate_edit(&draft) {
            self.state.profile.edit = EditPhase::Failed(e);
            return;
        }
        if self.state.account.as_ref().is_some_and(|a| a.steam_id.0 == 0) {
            self.state.notice = Some("Not available in the demo".into());
            self.state.profile.edit = EditPhase::Idle;
            return;
        }
        self.state.profile.edit = EditPhase::Saving;
        self.web_task(Some(draft));
    }

    pub(crate) fn profile_edit_cancel(&mut self) {
        self.state.profile.edit = EditPhase::Idle;
        self.state.profile.edit_original = None;
    }

    /// One web-session job: load the edit form (`None`) or save `Some(draft)`.
    fn web_task(&mut self, draft: Option<EditableProfile>) {
        let (Some(s), Some(hub)) = (&self.session, &self.hub) else {
            self.state.profile.edit = EditPhase::Failed("Not connected to Steam".into());
            return;
        };
        let (steam_id, token) = (s.account.steam_id.0, TokenCopy(s.refresh_token.clone()));
        let hub = hub.clone();
        let (tx, wake, current) = (self.tx.clone(), self.wake.clone(), self.generation.clone());
        let generation = current.load(Ordering::Relaxed);
        self.rt.spawn(async move {
            let send = |ev: ProfileEvent| {
                let (tx, wake) = (tx.clone(), wake.clone());
                async move {
                    let _ = tx.send(Event::Profile { generation, ev }).await;
                    wake();
                }
            };
            let saving = draft.is_some();
            let fail = |e: String| if saving { ProfileEvent::Saved(Err(e)) } else { ProfileEvent::EditLoaded(Err(e)) };
            let Some(online) = hub.online_now() else {
                return send(fail("Not connected to Steam".into())).await;
            };
            let access = match cm::generate_access_token(&online.conn, &token.0, steam_id).await {
                Ok(t) => t,
                Err(e) => return send(fail(format!("{}", redact::safe_error(&e, &[token.0.as_str()])))).await,
            };
            let ev = tokio::task::spawn_blocking(move || {
                let web = WebSession::new(steam_id, access);
                match draft {
                    Some(d) => ProfileEvent::Saved(web.save(&d).map_err(|e| format!("{e:#}"))),
                    None => ProfileEvent::EditLoaded(web.load().map_err(|e| format!("{e:#}"))),
                }
            })
            .await
            .unwrap_or_else(|_| fail("The request was interrupted".into()));
            send(ev).await;
        });
    }

    /// A made-up profile for the demo account and its friends.
    fn demo_profile(&self, id: SteamId) -> ProfileXml {
        let f = &self.state.friends;
        let name = f.friend(id).map(|x| x.name.clone()).or_else(|| f.persona_name().map(str::to_string));
        ProfileXml {
            steam_id: id.0,
            name: name.unwrap_or_else(|| "demo".into()),
            privacy: community::Privacy::Public,
            member_since: "March 3, 2012".into(),
            location: "Somewhere, Earth".into(),
            real_name: "Demo Person".into(),
            summary: "This is a demo profile.\nNothing here is real.".into(),
            hours_2wk: 12.5,
            most_played: vec![community::MostPlayed {
                name: "Dota 2".into(),
                app_id: 570,
                hours_2wk: 12.5,
                hours_total: 1234.0,
            }],
            ..Default::default()
        }
    }
}
