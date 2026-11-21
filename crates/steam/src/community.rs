//! Steam Community profiles: reading anyone's public profile and editing your own.
//!
//! Reading uses two public pages that need no login: `steamcommunity.com/profiles/<id>?xml=1` (name, summary,
//! location, privacy) and `steamcommunity.com/miniprofile/<account id>/json` (Steam level). Editing goes through
//! the community website's own form, which needs a web session: the cookie `steamLoginSecure` is the SteamID and
//! an access token, plus a random `sessionid` that must also be sent as a form field. Those values live only in
//! memory (see [`WebSession`]); errors are scrubbed with [`crate::redact`] before they leave this module.

use crate::http;
use crate::redact;
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::hash::{BuildHasher, Hasher};

const MAX_PAGE_BYTES: u64 = 2 * 1024 * 1024;
const COMMUNITY: &str = "https://steamcommunity.com";
const AVATAR_HOST: &str = "https://avatars.steamstatic.com";
/// Longest persona name / real name / summary Steam accepts (characters). Summary is 2000 on the site.
pub const MAX_NAME: usize = 32;
pub const MAX_REAL_NAME: usize = 64;
pub const MAX_SUMMARY: usize = 2000;

// ---- reading ----------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Privacy {
    Public,
    FriendsOnly,
    Private,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MostPlayed {
    pub name: String,
    pub app_id: u32,
    pub hours_2wk: f32,
    pub hours_total: f32,
}

/// What `?xml=1` says about a profile. Text fields are plain text (no HTML).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProfileXml {
    pub steam_id: u64,
    pub name: String,
    pub privacy: Privacy,
    pub state_message: String,
    /// 40 hex digits of the avatar picture; `None` = Steam's default.
    pub avatar_hash: Option<String>,
    pub custom_url: String,
    pub member_since: String,
    pub location: String,
    pub real_name: String,
    pub headline: String,
    pub summary: String,
    pub hours_2wk: f32,
    pub vac_banned: bool,
    pub limited: bool,
    pub most_played: Vec<MostPlayed>,
}

/// URL of an avatar picture in full size (184x184).
pub fn avatar_full_url(hash: &str) -> String {
    format!("{AVATAR_HOST}/{hash}_full.jpg")
}

/// The content of the first `<name>...</name>`, with a CDATA wrapper removed. `None` when absent.
fn tag(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&format!("</{name}>"))? + start;
    let inner = xml[start..end].trim();
    let inner = inner.strip_prefix("<![CDATA[").and_then(|r| r.strip_suffix("]]>")).unwrap_or(inner);
    Some(inner.to_string())
}

/// `xml` without every `<name>...</name>` block (nested blocks of other kinds would confuse [`tag`]).
fn without_block(xml: &str, name: &str) -> String {
    let (open, close) = (format!("<{name}>"), format!("</{name}>"));
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(a) = rest.find(&open) {
        out.push_str(&rest[..a]);
        match rest[a..].find(&close) {
            Some(b) => rest = &rest[a + b + close.len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Decode the few entities that show up in profile text.
fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

/// Turn the HTML of a profile summary into plain text: `<br>` becomes a new line, other tags vanish.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut chars = html.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '<' {
            out.push(c);
            continue;
        }
        let mut tag = String::new();
        for t in chars.by_ref() {
            if t == '>' {
                break;
            }
            tag.push(t);
        }
        let name = tag.trim_start_matches('/').split([' ', '/']).next().unwrap_or_default().to_ascii_lowercase();
        if name == "br" || (name == "p" && tag.starts_with('/')) {
            out.push('\n');
        }
    }
    let text = decode_entities(&out);
    // Collapse the runs of blank lines the markup leaves behind.
    let mut cleaned = String::new();
    let mut blank = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        cleaned.push_str(line.trim_end());
        cleaned.push('\n');
    }
    cleaned.trim().to_string()
}

fn hash_from_avatar_url(url: &str) -> Option<String> {
    let file = url.rsplit('/').next()?;
    let stem = file.split(['.', '_']).next()?;
    (stem.len() == 40 && stem.bytes().all(|b| b.is_ascii_hexdigit())).then(|| stem.to_ascii_lowercase())
}

fn app_id_from_link(link: &str) -> u32 {
    link.split("/app/").nth(1).and_then(|r| r.split(['/', '?']).next()).and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// Parse the answer of `steamcommunity.com/profiles/<id>?xml=1`. An error page (`<response><error>`) or a
/// document without a SteamID is an error.
pub fn parse_profile_xml(xml: &str) -> Result<ProfileXml, String> {
    if let Some(e) = tag(xml, "error") {
        return Err(if e.is_empty() { "Steam says this profile doesn't exist".into() } else { decode_entities(&e) });
    }
    let most = tag(xml, "mostPlayedGames").unwrap_or_default();
    let top = without_block(&without_block(xml, "groups"), "mostPlayedGames");
    let steam_id: u64 = tag(&top, "steamID64")
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| "The profile page had no SteamID".to_string())?;
    let text = |name: &str| tag(&top, name).map(|v| decode_entities(&v)).unwrap_or_default();
    let privacy = match tag(&top, "privacyState").unwrap_or_default().as_str() {
        "public" => Privacy::Public,
        "friendsonly" => Privacy::FriendsOnly,
        "private" => Privacy::Private,
        _ => Privacy::Unknown,
    };
    let mut most_played = Vec::new();
    let mut rest = most.as_str();
    while let Some(a) = rest.find("<mostPlayedGame>") {
        let Some(b) = rest[a..].find("</mostPlayedGame>") else { break };
        let block = &rest[a..a + b];
        rest = &rest[a + b + "</mostPlayedGame>".len()..];
        let num = |name: &str| {
            tag(block, name).map(|v| v.replace(',', "")).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0)
        };
        most_played.push(MostPlayed {
            name: tag(block, "gameName").map(|v| decode_entities(&v)).unwrap_or_default(),
            app_id: app_id_from_link(&tag(block, "gameLink").unwrap_or_default()),
            hours_2wk: num("hoursPlayed"),
            hours_total: num("hoursOnRecord"),
        });
    }
    Ok(ProfileXml {
        steam_id,
        name: text("steamID"),
        privacy,
        state_message: html_to_text(&text("stateMessage")),
        avatar_hash: tag(&top, "avatarFull").and_then(|u| hash_from_avatar_url(&u)),
        custom_url: text("customURL"),
        member_since: text("memberSince"),
        location: text("location"),
        real_name: text("realname"),
        headline: text("headline"),
        summary: html_to_text(&tag(&top, "summary").unwrap_or_default()),
        hours_2wk: tag(&top, "hoursPlayed2Wk").and_then(|v| v.parse().ok()).unwrap_or(0.0),
        vac_banned: tag(&top, "vacBanned").is_some_and(|v| v == "1"),
        limited: tag(&top, "isLimitedAccount").is_some_and(|v| v == "1"),
        most_played,
    })
}

/// Fetch and parse a public profile. Blocking; call from a worker thread.
pub fn fetch_profile_xml(steam_id: u64) -> Result<ProfileXml> {
    let url = format!("{COMMUNITY}/profiles/{steam_id}?xml=1");
    let resp = http::agent().get(&url).call().context("profile request failed")?;
    let body =
        resp.into_body().with_config().limit(MAX_PAGE_BYTES).read_to_string().context("profile page unreadable")?;
    parse_profile_xml(&body).map_err(|e| anyhow::anyhow!(e))
}

/// The Steam level from `miniprofile/<account id>/json`.
pub fn parse_level(json: &str) -> Option<u32> {
    let v: Value = serde_json::from_str(json).ok()?;
    u32::try_from(v.get("level")?.as_u64()?).ok()
}

/// Fetch the Steam level of an account (`None` when the page has none). Blocking.
pub fn fetch_level(steam_id: u64) -> Result<Option<u32>> {
    let url = format!("{COMMUNITY}/miniprofile/{}/json", steam_id & 0xFFFF_FFFF);
    let resp = http::agent().get(&url).call().context("level request failed")?;
    let body =
        resp.into_body().with_config().limit(MAX_PAGE_BYTES).read_to_string().context("level page unreadable")?;
    Ok(parse_level(&body))
}

// ---- editing ----------------------------------------------------------------------------------------

/// The profile fields the edit form carries.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EditableProfile {
    pub persona_name: String,
    pub real_name: String,
    pub summary: String,
    pub country: String,
    pub state: String,
    pub city: String,
    pub custom_url: String,
}

fn json_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// Read the current values from the edit page: they sit as JSON in the `data-profile-edit` attribute of the
/// `profile_edit_config` element. Fields the form carries but we don't edit (location, custom URL) are kept so
/// saving doesn't erase them.
pub fn parse_edit_config(html: &str) -> Result<EditableProfile, String> {
    const KEY: &str = "data-profile-edit=\"";
    let start = html.find(KEY).ok_or("The edit page had no profile data")? + KEY.len();
    let end = html[start..].find('"').ok_or("The edit page was cut off")? + start;
    let v: Value = serde_json::from_str(&decode_entities(&html[start..end])).map_err(|_| "Malformed profile data")?;
    let name = json_text(&v["strPersonaName"]);
    if name.is_empty() {
        return Err("Malformed profile data".into());
    }
    let loc = &v["LocationData"];
    Ok(EditableProfile {
        persona_name: name,
        real_name: json_text(&v["strRealName"]),
        summary: json_text(&v["strSummary"]),
        country: json_text(&loc["locCountryCode"]),
        state: json_text(&loc["locStateCode"]),
        city: json_text(&loc["locCityCode"]),
        custom_url: json_text(&v["strCustomURL"]),
    })
}

/// Check what the user typed before it is sent.
pub fn validate_edit(p: &EditableProfile) -> Result<(), String> {
    if p.persona_name.trim().is_empty() {
        return Err("The profile name can't be empty".into());
    }
    if p.persona_name.chars().count() > MAX_NAME {
        return Err(format!("The profile name can have at most {MAX_NAME} characters"));
    }
    if p.real_name.chars().count() > MAX_REAL_NAME {
        return Err(format!("The real name can have at most {MAX_REAL_NAME} characters"));
    }
    if p.summary.chars().count() > MAX_SUMMARY {
        return Err(format!("The summary can have at most {MAX_SUMMARY} characters"));
    }
    Ok(())
}

/// Percent-encode for `application/x-www-form-urlencoded` (everything but unreserved characters).
pub fn form_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The body of the save request, in the field order and names the site's own form uses.
pub fn build_edit_form(session_id: &str, p: &EditableProfile) -> String {
    let fields: [(&str, &str); 16] = [
        ("sessionID", session_id),
        ("type", "profileSave"),
        ("weblink_1_title", ""),
        ("weblink_1_url", ""),
        ("weblink_2_title", ""),
        ("weblink_2_url", ""),
        ("weblink_3_title", ""),
        ("weblink_3_url", ""),
        ("personaName", p.persona_name.trim()),
        ("real_name", p.real_name.trim()),
        ("summary", p.summary.trim()),
        ("country", p.country.as_str()),
        ("state", p.state.as_str()),
        ("city", p.city.as_str()),
        ("customURL", p.custom_url.as_str()),
        ("json", "1"),
    ];
    fields.iter().map(|(k, v)| format!("{k}={}", form_encode(v))).collect::<Vec<_>>().join("&")
}

/// 24 random hex digits for the `sessionid` cookie and form field.
pub fn random_session_id() -> String {
    let state = std::collections::hash_map::RandomState::new();
    let mut out = String::with_capacity(24);
    for i in 0..3u64 {
        let mut h = state.build_hasher();
        h.write_u64(i);
        out.push_str(&format!("{:08x}", h.finish() as u32));
    }
    out
}

/// Cookies for the community website. Never printed (`Debug` hides the token) and never written to disk.
pub struct WebSession {
    pub steam_id: u64,
    token: String,
    pub session_id: String,
}

impl std::fmt::Debug for WebSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSession").field("steam_id", &self.steam_id).finish_non_exhaustive()
    }
}

impl Drop for WebSession {
    fn drop(&mut self) {
        // SAFETY: zero bytes are valid UTF-8.
        unsafe { self.token.as_bytes_mut() }.fill(0);
    }
}

impl WebSession {
    pub fn new(steam_id: u64, access_token: String) -> Self {
        Self { steam_id, token: access_token, session_id: random_session_id() }
    }

    fn cookie(&self) -> String {
        format!("steamLoginSecure={}%7C%7C{}; sessionid={}", self.steam_id, self.token, self.session_id)
    }

    fn scrub(&self, e: anyhow::Error) -> anyhow::Error {
        redact::safe_error(&e, &[self.token.as_str(), self.cookie().as_str()])
    }

    fn edit_url(&self) -> String {
        format!("{COMMUNITY}/profiles/{}/edit", self.steam_id)
    }

    /// The current editable values. Blocking.
    pub fn load(&self) -> Result<EditableProfile> {
        let url = format!("{}/info", self.edit_url());
        let get = || -> Result<String> {
            let resp =
                http::agent().get(&url).header("Cookie", self.cookie()).call().context("edit page request failed")?;
            resp.into_body().with_config().limit(MAX_PAGE_BYTES).read_to_string().context("edit page unreadable")
        };
        let body = get().map_err(|e| self.scrub(e))?;
        parse_edit_config(&body).map_err(|e| anyhow::anyhow!(e))
    }

    /// Save `p`. Blocking.
    pub fn save(&self, p: &EditableProfile) -> Result<()> {
        validate_edit(p).map_err(|e| anyhow::anyhow!(e))?;
        let body = build_edit_form(&self.session_id, p);
        let post = || -> Result<String> {
            let resp = http::agent()
                .post(&self.edit_url())
                .header("Cookie", self.cookie())
                .header("Content-Type", "application/x-www-form-urlencoded")
                .header("Referer", format!("{}/info", self.edit_url()))
                .send(body.as_str())
                .context("save request failed")?;
            resp.into_body().with_config().limit(MAX_PAGE_BYTES).read_to_string().context("save answer unreadable")
        };
        let answer = post().map_err(|e| self.scrub(e))?;
        check_save_answer(&answer)
    }
}

/// The save answer is JSON: `{"success":1}` or `{"success":..,"errmsg":"..."}`.
pub fn check_save_answer(body: &str) -> Result<()> {
    let v: Value = serde_json::from_str(body).context("Steam sent an unexpected answer")?;
    if v["success"].as_i64() == Some(1) {
        return Ok(());
    }
    match v["errmsg"].as_str().filter(|m| !m.is_empty()) {
        Some(m) => bail!("{}", decode_entities(m)),
        None => bail!("Steam did not accept the changes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><profile>
	<steamID64>76561197960435530</steamID64>
	<steamID><![CDATA[Robin]]></steamID>
	<onlineState>offline</onlineState>
	<stateMessage><![CDATA[Offline]]></stateMessage>
	<privacyState>public</privacyState>
	<visibilityState>3</visibilityState>
	<avatarIcon><![CDATA[https://avatars.fastly.steamstatic.com/81b5478529dce13bf24b55ac42c1af7058aaf7a9.jpg]]></avatarIcon>
	<avatarFull><![CDATA[https://avatars.fastly.steamstatic.com/81b5478529dce13bf24b55ac42c1af7058aaf7a9_full.jpg]]></avatarFull>
	<vacBanned>0</vacBanned>
	<isLimitedAccount>0</isLimitedAccount>
	<customURL><![CDATA[robinwalker]]></customURL>
	<memberSince>September 12, 2003</memberSince>
	<hoursPlayed2Wk>1.5</hoursPlayed2Wk>
	<headline><![CDATA[]]></headline>
	<location><![CDATA[Bellevue, Washington, United States]]></location>
	<realname><![CDATA[Robin Walker]]></realname>
	<summary><![CDATA[Hello &amp; welcome.<br><br>See <a class="bb_link" href="https://x.y/" >https://x.y/</a> now]]></summary>
	<mostPlayedGames>
		<mostPlayedGame>
			<gameName><![CDATA[Team Fortress 2]]></gameName>
			<gameLink><![CDATA[https://steamcommunity.com/app/440]]></gameLink>
			<hoursPlayed>1.5</hoursPlayed>
			<hoursOnRecord>1,234.5</hoursOnRecord>
		</mostPlayedGame>
	</mostPlayedGames>
	<groups>
		<group isPrimary="1">
			<groupID64>103582791429521412</groupID64>
			<groupName><![CDATA[Valve]]></groupName>
			<headline><![CDATA[VALVE]]></headline>
			<summary><![CDATA[Group summary]]></summary>
		</group>
	</groups>
</profile>"#;

    #[test]
    fn parses_a_public_profile() {
        let p = parse_profile_xml(SAMPLE).unwrap();
        assert_eq!(p.steam_id, 76_561_197_960_435_530);
        assert_eq!((p.name.as_str(), p.privacy), ("Robin", Privacy::Public));
        assert_eq!(p.real_name, "Robin Walker");
        assert_eq!(p.location, "Bellevue, Washington, United States");
        assert_eq!(p.custom_url, "robinwalker");
        assert_eq!(p.summary, "Hello & welcome.\n\nSee https://x.y/ now");
        assert_eq!(p.avatar_hash.as_deref(), Some("81b5478529dce13bf24b55ac42c1af7058aaf7a9"));
        assert_eq!(p.hours_2wk, 1.5);
        assert!(!p.vac_banned && !p.limited);
        assert_eq!(
            p.most_played,
            [MostPlayed { name: "Team Fortress 2".into(), app_id: 440, hours_2wk: 1.5, hours_total: 1234.5 }]
        );
    }

    #[test]
    fn group_text_does_not_leak_into_the_profile() {
        let p = parse_profile_xml(SAMPLE).unwrap();
        assert!(!p.summary.contains("Group summary"));
        assert_ne!(p.headline, "VALVE");
    }

    #[test]
    fn a_private_profile_has_only_the_basics() {
        let xml = "<profile><steamID64>76561197960287930</steamID64><steamID><![CDATA[Rabscuttle]]></steamID>\
                   <privacyState>friendsonly</privacyState><avatarFull><![CDATA[https://a/b.jpg]]></avatarFull></profile>";
        let p = parse_profile_xml(xml).unwrap();
        assert_eq!((p.name.as_str(), p.privacy), ("Rabscuttle", Privacy::FriendsOnly));
        assert!(p.summary.is_empty() && p.real_name.is_empty() && p.avatar_hash.is_none());
    }

    #[test]
    fn error_pages_and_garbage_are_errors() {
        assert!(
            parse_profile_xml(
                "<response><error><![CDATA[The specified profile could not be found.]]></error></response>"
            )
            .unwrap_err()
            .contains("could not be found")
        );
        assert!(parse_profile_xml("<html></html>").is_err());
    }

    #[test]
    fn html_becomes_text() {
        assert_eq!(html_to_text("a<br>b &lt;3<br><br><br>c"), "a\nb <3\n\nc");
        assert_eq!(html_to_text("<a href=\"x\">link</a>"), "link");
    }

    #[test]
    fn levels_parse() {
        assert_eq!(parse_level(r#"{"level":157,"persona_name":"McKay"}"#), Some(157));
        assert_eq!(parse_level("{}"), None);
        assert_eq!(parse_level("nope"), None);
    }

    #[test]
    fn edit_config_is_read_from_the_attribute() {
        let html = r#"<div id="profile_edit_config" data-profile-edit="{&quot;strPersonaName&quot;:&quot;Ann &amp; Co&quot;,&quot;strRealName&quot;:&quot;Ann&quot;,&quot;strSummary&quot;:&quot;Hi&quot;,&quot;strCustomURL&quot;:&quot;ann&quot;,&quot;LocationData&quot;:{&quot;locCountryCode&quot;:&quot;DE&quot;,&quot;locStateCode&quot;:&quot;BE&quot;,&quot;locCityCode&quot;:1234}}"></div>"#;
        let p = parse_edit_config(html).unwrap();
        assert_eq!(p.persona_name, "Ann & Co");
        assert_eq!((p.real_name.as_str(), p.summary.as_str(), p.custom_url.as_str()), ("Ann", "Hi", "ann"));
        assert_eq!((p.country.as_str(), p.state.as_str(), p.city.as_str()), ("DE", "BE", "1234"));
        assert!(parse_edit_config("<html></html>").is_err());
    }

    #[test]
    fn the_edit_form_is_built_and_encoded() {
        let p = EditableProfile {
            persona_name: " Ann & Bo ".into(),
            real_name: "Änn".into(),
            summary: "line1\nline2 = 100%".into(),
            country: "DE".into(),
            ..Default::default()
        };
        let body = build_edit_form("abc123", &p);
        assert!(body.starts_with("sessionID=abc123&type=profileSave&weblink_1_title=&"));
        assert!(body.contains("&personaName=Ann+%26+Bo&"));
        assert!(body.contains("&real_name=%C3%84nn&"));
        assert!(body.contains("&summary=line1%0Aline2+%3D+100%25&"));
        assert!(body.contains("&country=DE&state=&city=&customURL=&json=1"));
        assert!(body.ends_with("json=1"));
    }

    #[test]
    fn edits_are_validated() {
        let ok = EditableProfile { persona_name: "Ann".into(), ..Default::default() };
        assert!(validate_edit(&ok).is_ok());
        assert!(validate_edit(&EditableProfile::default()).is_err());
        let long = EditableProfile { persona_name: "x".repeat(MAX_NAME + 1), ..Default::default() };
        assert!(validate_edit(&long).is_err());
        let long = EditableProfile { summary: "x".repeat(MAX_SUMMARY + 1), ..ok.clone() };
        assert!(validate_edit(&long).is_err());
    }

    #[test]
    fn save_answers_are_checked() {
        assert!(check_save_answer(r#"{"success":1}"#).is_ok());
        let e = check_save_answer(r#"{"success":2,"errmsg":"Name &amp; too long"}"#).unwrap_err();
        assert_eq!(e.to_string(), "Name & too long");
        assert!(check_save_answer("<html>").is_err());
    }

    #[test]
    fn session_ids_are_24_hex_digits_and_the_token_stays_out_of_debug() {
        let id = random_session_id();
        assert_eq!(id.len(), 24);
        assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
        let s = WebSession::new(76_561_197_960_435_530, "secret-token-value".into());
        assert!(!format!("{s:?}").contains("secret"));
        assert!(s.cookie().starts_with("steamLoginSecure=76561197960435530%7C%7Csecret-token-value; sessionid="));
    }
}
