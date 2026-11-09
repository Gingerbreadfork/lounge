//! Simkl: what you paused, the next episode of every show and anime you are watching, what
//! you plan to watch and what airs this week, whichever service they live on. Simkl knows the
//! title, not the app, so selecting an item searches for it: Lounge's own search, or the TV's.
//! Sign-in is Simkl's device code, shown on the plugin's page until it has been entered at
//! simkl.com/pin. Lists are read only when Simkl says they changed.
use lounge_plugin_sdk::{self as sdk, host, parse_iso8601, Action, HttpRequest, Item, ItemKind, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

pub struct Simkl;

const API: &str = "https://api.simkl.com";
const IMAGES: &str = "https://simkl.in";
const CALENDAR: &str = "https://data.simkl.in/calendar/v2";
const APP_NAME: &str = "lounge";
const TOKEN_KEY: &str = "token";
const DEVICE_KEY: &str = "device";
const ACTIVITIES_KEY: &str = "activities";
const PLAYBACK_KEY: &str = "playback";
const ART_KEY: &str = "art";
const CALENDAR_KEY: &str = "calendar";
const PAUSE_KEY: &str = "pause_until";
const PAUSE: &str = "Simkl is asking for a pause between requests. It tries again on the next refresh.";
const UNKNOWN_CLIENT: &str = "Simkl does not know this Client ID, or has paused it. Check it against your app at simkl.com/settings/developer.";
/// Simkl's device endpoint answers the same for an unknown id and for an app made under its
/// older sign-in, which cannot be moved over.
const NOT_A_TV_APP: &str = "Simkl does not accept this Client ID for signing in. Apps made before Simkl's new sign-in do not work: at simkl.com/settings/developer create an app of the type TV, devices and command line, and use its Client ID.";
const SIGN_IN_AGAIN: &str = "Simkl no longer accepts this sign-in. Press Refresh now for a new code.";
const RENEW_LATER: &str = "Could not renew the Simkl sign-in. It tries again on the next refresh.";
/// Most shows in Up next, and most paused items.
const UP_NEXT: usize = 12;
const PAUSED: usize = 20;
const PLANNED: usize = 24;
const AIRINGS_PER_SHOW: usize = 2;
/// Most entries kept from a list, newest first, so a long list fits the key-value store.
const KEPT_WATCHING: usize = 400;
const KEPT_PLANNED: usize = 300;
/// Most airings kept from the calendar, soonest first.
const KEPT_AIRINGS: usize = 400;
/// What one key-value entry may hold, under the host's 256 KB.
const KV_ROOM: usize = 200 * 1024;
/// Title summaries looked up for their artwork per refresh, and how many are remembered.
const ART_LOOKUPS: usize = 12;
const ART_KEPT: usize = 300;
const ART_FRESH_MS: i64 = 30 * DAY_MS;
const DAY_MS: i64 = 86_400_000;
const WEEK_MS: i64 = 7 * DAY_MS;
const HOUR_MS: i64 = 3_600_000;
/// A list or a calendar file is read whole, so a slow answer ends early.
const FETCH_TIMEOUT_MS: u32 = 15_000;

impl sdk::Plugin for Simkl {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.simkl".into(),
            name: "Simkl".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Continue watching, up next, plan to watch and the week's new episodes from Simkl, for shows, anime and films across every service.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Search],
            fetch_hosts: vec!["api.simkl.com".into(), "simkl.in".into(), "data.simkl.in".into()],
            // Simkl asks TV apps for 15 to 30 minutes, and counts every call against the day.
            refresh_minutes: 20,
            rows: vec![
                RowSpec { id: "upnext".into(), title: "Up next on Simkl".into(), style: RowStyle::Landscape },
                RowSpec { id: "airing".into(), title: "Coming up on Simkl".into(), style: RowStyle::Landscape },
                RowSpec { id: "planned".into(), title: "Plan to watch on Simkl".into(), style: RowStyle::Poster },
            ],
            settings: vec![
                SettingSpec {
                    key: "client_id".into(),
                    label: "Client ID".into(),
                    kind: SettingKind::Text,
                    required: true,
                    help: Some("At simkl.com/settings/developer create an app of the type TV, devices and command line, any name, and copy its Client ID. Nothing else is needed; the sign-in code then appears on this page.".into()),
                    default: None,
                },
                SettingSpec { key: "upnext".into(), label: "Up next row".into(), kind: SettingKind::Toggle, required: false, help: Some("The next episode of each show and anime you are watching.".into()), default: Some("true".into()) },
                SettingSpec { key: "airing".into(), label: "Coming up row".into(), kind: SettingKind::Toggle, required: false, help: Some("Episodes airing in the next week of the shows and anime you watch or plan to watch.".into()), default: Some("true".into()) },
                SettingSpec { key: "planned".into(), label: "Plan to watch row".into(), kind: SettingKind::Toggle, required: false, help: None, default: Some("true".into()) },
                SettingSpec {
                    key: "tv_search".into(),
                    label: "Open with the TV's own search".into(),
                    kind: SettingKind::Toggle,
                    required: false,
                    help: Some("Off: Lounge searches your apps and plugins for the title. On: the TV's search, which on Google TV lists where it streams. A result of Lounge's search always opens the TV's search.".into()),
                    default: Some("false".into()),
                },
                SettingSpec {
                    key: "catalogue".into(),
                    label: "Search all of Simkl".into(),
                    kind: SettingKind::Toggle,
                    required: false,
                    help: Some("Off: Lounge's search finds titles on your Simkl lists, with no request. On: it asks Simkl's whole catalogue, which counts against Simkl's daily allowance for your account.".into()),
                    default: Some("false".into()),
                },
            ],
            homepage: Some("https://simkl.com".into()),
            self_signed_servers: false,
            for_apps: Vec::new(),
            sent_to: Default::default(),
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let mut client = Client::connect(ctx.now_ms)?;
        let now = ctx.now_ms;
        let open = opener();
        let want = |key: &str| host::setting(key).map(|v| v != "false").unwrap_or(true);
        let (upnext, airing, planned) = (want("upnext"), want("airing"), want("planned"));

        // What changed since last time decides what is read; an unchanged list is kept as it
        // was, and while Simkl has the day's allowance paused nothing is asked at all.
        let paused_until: Option<i64> = host::kv_get_string(PAUSE_KEY).and_then(|v| v.parse().ok()).filter(|&t| t > now);
        let fresh: Option<Activities> = match paused_until {
            Some(until) => {
                sdk::warn!("Simkl's daily allowance is used up; using what is stored until {until}");
                None
            }
            None => Some(client.get_json("/sync/activities", "")?),
        };
        let known: Option<Activities> = host::kv_get(ACTIVITIES_KEY).and_then(|v| serde_json::from_slice(&v).ok());
        let known = known.as_ref();
        let mut lists = Lists::default();
        let mut read = 0;
        let mut unread = Vec::new();
        for bucket in Bucket::ALL {
            let wanted = match bucket.status {
                Status::Watching => upnext || airing,
                Status::PlanToWatch => planned || airing,
            };
            let moved = fresh.as_ref().is_some_and(|f| known.is_none_or(|k| bucket.stamp(f) != bucket.stamp(k) || bucket.kind.removed(f) != bucket.kind.removed(k)));
            if !wanted {
                if moved {
                    unread.push(bucket);
                }
                continue;
            }
            let stored = host::kv_get(&bucket.key()).and_then(|v| serde_json::from_slice::<Vec<Title>>(&v).ok());
            let titles = match (stored, fresh.is_some()) {
                (Some(t), _) if !moved => t,
                (None, false) => Vec::new(),
                _ => {
                    let titles = client.list(bucket)?;
                    kv_set_within(&bucket.key(), &titles, |t| t.pop().is_some());
                    read += 1;
                    titles
                }
            };
            lists.put(bucket, titles);
        }

        let stored: Option<Vec<Playback>> = host::kv_get(PLAYBACK_KEY).and_then(|v| serde_json::from_slice(&v).ok());
        let playback_moved = fresh.as_ref().is_some_and(|f| known.is_none_or(|k| Domain::playback_stamps(f) != Domain::playback_stamps(k)));
        let playback = match (stored, fresh.is_some()) {
            (Some(p), _) if !playback_moved => p,
            (None, false) => Vec::new(),
            _ => {
                let mut p: Vec<Playback> = client.get_json("/sync/playback", "")?;
                p.retain(|p| p.title().is_some());
                p.sort_by_key(|p| std::cmp::Reverse(p.paused_at.as_deref().and_then(parse_iso8601).unwrap_or(0)));
                p.truncate(PAUSED);
                kv_set_within(PLAYBACK_KEY, &p, |p| p.pop().is_some());
                read += 1;
                p
            }
        };
        // What was read is the state to compare with next time. A list that changed while its
        // rows were off keeps no stamp, so it is read once they are back on.
        if let Some(mut f) = fresh {
            for bucket in unread {
                bucket.forget_stamp(&mut f);
            }
            host::kv_set(ACTIVITIES_KEY, &serde_json::to_vec(&f).unwrap_or_default());
        }

        let mut art = ArtCache::load(now);
        let mut items = Vec::new();
        for p in &playback {
            if let Some(item) = resume_item(p, &lists, &mut art, &client, &open) {
                items.push(item);
            }
        }
        let paused = items.len();
        let paused_episodes: HashSet<(u64, u32, u32)> = playback.iter().filter_map(|p| Some((p.title()?.ids.simkl?, p.episode.as_ref()?.season, p.episode.as_ref()?.number))).collect();
        let paused_titles: HashSet<u64> = playback.iter().filter_map(|p| p.title()?.ids.simkl).collect();

        let mut up_next = 0;
        let mut waiting: HashSet<(u64, Option<u32>, u32)> = HashSet::new();
        if upnext {
            let mut watching: Vec<&Title> = lists.watching().filter(|t| t.next.is_some()).collect();
            watching.sort_by_key(|t| std::cmp::Reverse(t.last_watched_ms.unwrap_or(0)));
            for t in watching {
                if up_next >= UP_NEXT {
                    break;
                }
                let Some(next) = t.next.as_ref().filter(|n| !paused_episodes.contains(&(t.id, n.season.unwrap_or(1), n.episode))) else { continue };
                let look = art.lookup(t.id, t.kind, &client);
                items.push(up_next_item(t, next, look, (UP_NEXT - up_next) as i32, now, &open));
                waiting.insert((t.id, next.season, next.episode));
                up_next += 1;
            }
        }

        let mut airing_count = 0;
        if airing {
            let mut calendar = Calendar::load();
            for kind in [TitleKind::Show, TitleKind::Anime] {
                let followed: HashSet<u64> = lists.all().filter(|t| t.kind == kind).map(|t| t.id).collect();
                match calendar.update(kind, &followed, now) {
                    Ok(()) => {}
                    Err(e) if e.contains("budget") => break,
                    Err(e) => sdk::warn!("calendar: {e}"),
                }
            }
            calendar.save();
            let mut soon: Vec<(&Airing, &Followed)> = calendar
                .feeds
                .values()
                .flat_map(|f| f.entries.iter().filter_map(|a| Some((a, f.shows.get(&a.id)?))))
                // An episode already waiting in Up next is not coming up as well.
                .filter(|(a, _)| a.date_ms >= now && a.date_ms < now + WEEK_MS && !waiting.contains(&(a.id, a.season, a.episode)))
                .collect();
            soon.sort_by_key(|(a, _)| a.date_ms);
            soon.dedup_by_key(|(a, _)| (a.id, a.season, a.episode));
            // A daily show would fill the row on its own, so each keeps its next two.
            let mut seen: HashMap<u64, usize> = HashMap::new();
            soon.retain(|(a, _)| {
                let n = seen.entry(a.id).or_default();
                *n += 1;
                *n <= AIRINGS_PER_SHOW
            });
            let n = soon.len();
            for (i, (a, show)) in soon.iter().enumerate() {
                items.push(airing_item(a, show, now, ctx.utc_offset_minutes, (n - i) as i32, &open));
                airing_count += 1;
            }
        }

        let mut planned_count = 0;
        if planned {
            let mut plans: Vec<&Title> = lists.planned().filter(|t| !paused_titles.contains(&t.id)).collect();
            plans.sort_by_key(|t| std::cmp::Reverse(t.added_ms.unwrap_or(0)));
            for (i, t) in plans.iter().take(PLANNED).enumerate() {
                items.push(planned_item(t, now, (PLANNED - i) as i32, &open));
                planned_count += 1;
            }
        }
        art.save();
        sdk::dedup_ids(&mut items);
        sdk::info!("{read} lists read, {paused} paused, {up_next} up next, {airing_count} coming up, {planned_count} planned");
        Ok(items)
    }

    /// Titles from the stored lists, or from Simkl's catalogue when asked for. Results open
    /// the TV's search, since a result that only searches again is none. Search never signs in
    /// or renews, so a sign-in in progress is left to the refresh.
    fn search(query: &str, limit: u32) -> Result<Vec<Item>, String> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let limit = limit.clamp(1, 50) as usize;
        let open = |title: &str| tv_search_action(title);
        if host::setting("catalogue").map(|v| v == "true").unwrap_or(false) {
            let Some(mut client) = Client::signed_in() else { return Ok(Vec::new()) };
            let found: Vec<Found> = match client.get_json("/search/all", &format!("&q={}&limit={limit}", uri_encode(query))) {
                Ok(f) => f,
                Err(e) if e == SIGN_IN_AGAIN || e == RENEW_LATER => return Ok(Vec::new()),
                Err(e) => return Err(e),
            };
            return Ok(found.iter().filter_map(|f| found_item(f, &open)).take(limit).collect());
        }
        let needle = query.to_lowercase();
        let mut found = Vec::new();
        for bucket in Bucket::ALL {
            let Some(titles) = host::kv_get(&bucket.key()).and_then(|v| serde_json::from_slice::<Vec<Title>>(&v).ok()) else { continue };
            found.extend(titles.into_iter().filter(|t| t.title.to_lowercase().contains(&needle)).map(|t| listed_item(&t, &open)));
        }
        sdk::dedup_ids(&mut found);
        found.truncate(limit);
        Ok(found)
    }
}

sdk::export_plugin!(Simkl);

/// How a title is opened: Lounge's search, or the TV's own.
fn opener() -> Box<dyn Fn(&str) -> Action> {
    let tv_search = host::setting("tv_search").map(|v| v == "true").unwrap_or(false);
    Box::new(move |title: &str| if tv_search { tv_search_action(title) } else { Action::search(title) })
}

/// Stores `value` under `key`, dropping from it with `shrink` until it fits the room one entry
/// has; a store that is refused would end the whole refresh.
fn kv_set_within<T: Serialize + Clone>(key: &str, value: &[T], mut shrink: impl FnMut(&mut Vec<T>) -> bool) {
    let mut kept = value.to_vec();
    loop {
        let bytes = serde_json::to_vec(&kept).unwrap_or_default();
        if bytes.len() <= KV_ROOM || !shrink(&mut kept) {
            host::kv_set(key, &bytes);
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Signing in: Simkl's device code, one poll per refresh
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct Token {
    access: String,
    refresh: String,
    expires_ms: i64,
    /// The Client ID the token was issued to, so another one starts afresh.
    #[serde(default)]
    client_id: String,
}

impl Token {
    /// Renewal is due a day before the week-long token ends, as Simkl asks.
    fn due(&self, now_ms: i64) -> bool {
        self.expires_ms - now_ms < DAY_MS
    }

    fn load(client_id: &str) -> Option<Token> {
        let t: Token = host::kv_get(TOKEN_KEY).and_then(|v| serde_json::from_slice(&v).ok())?;
        if !t.client_id.is_empty() && t.client_id != client_id {
            sdk::info!("the Client ID changed; signing in afresh");
            forget_account();
            return None;
        }
        Some(t)
    }

    fn save(&self) {
        host::kv_set(TOKEN_KEY, serde_json::to_string(self).unwrap_or_default().as_bytes());
    }

    /// Marks the token spent, so the next refresh renews it with the refresh token.
    fn expire(&mut self) {
        self.expires_ms = 0;
        self.save();
    }
}

/// Drops everything kept about the signed-in account, for a sign-in that starts over.
fn forget_account() {
    host::kv_delete(TOKEN_KEY);
    host::kv_delete(DEVICE_KEY);
    host::kv_delete(ACTIVITIES_KEY);
    host::kv_delete(PLAYBACK_KEY);
    host::kv_delete(CALENDAR_KEY);
    host::kv_delete(PAUSE_KEY);
    for bucket in Bucket::ALL {
        host::kv_delete(&bucket.key());
    }
}

#[derive(Serialize, Deserialize)]
struct Device {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_ms: i64,
}

#[derive(Deserialize)]
struct DeviceAnswer {
    device_code: String,
    user_code: String,
    #[serde(default)]
    verification_uri: Option<String>,
    #[serde(default)]
    verification_url: Option<String>,
    expires_in: i64,
}

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

#[derive(Deserialize, Default)]
struct ApiError {
    #[serde(default)]
    error: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

fn error_of(body: &str) -> ApiError {
    serde_json::from_str(body).unwrap_or_default()
}

struct Client {
    client_id: String,
    token: Token,
}

impl Client {
    /// A signed-in client, or the one sentence the plugin's page should show to get there.
    fn connect(now_ms: i64) -> Result<Client, String> {
        let client_id = host::required_setting("client_id", "Client ID")?.trim().to_string();
        let token = match Token::load(&client_id) {
            Some(t) if !t.due(now_ms) => t,
            Some(t) => match renew(&client_id, &t, now_ms) {
                Ok(fresh) => fresh,
                Err(Renew::Refused) => {
                    forget_account();
                    device_step(&client_id, now_ms)?
                }
                // Anything else may pass, so the current token serves while it lasts.
                Err(Renew::Failed(e)) if t.expires_ms > now_ms => {
                    sdk::warn!("could not renew the sign-in ({e}); using the current one");
                    t
                }
                Err(Renew::Failed(e)) => {
                    sdk::warn!("could not renew the sign-in ({e})");
                    return Err(RENEW_LATER.into());
                }
            },
            None => device_step(&client_id, now_ms)?,
        };
        Ok(Client { client_id, token })
    }

    /// The client already signed in, if there is one, without signing in or renewing.
    fn signed_in() -> Option<Client> {
        let client_id = host::setting("client_id")?.trim().to_string();
        let token = Token::load(&client_id).filter(|t| t.expires_ms > host::now_ms())?;
        Some(Client { client_id, token })
    }

    fn url(&self, path: &str, query: &str) -> String {
        format!("{API}{path}?{}{query}", app_query(&self.client_id))
    }

    fn request(&self, path: &str, query: &str, signed: bool) -> HttpRequest {
        let mut r = HttpRequest { timeout_ms: FETCH_TIMEOUT_MS, ..HttpRequest::get(self.url(path, query)) }.header("User-Agent", &user_agent()).header("Accept", "application/json");
        if signed {
            r = r.header("Authorization", &format!("Bearer {}", self.token.access));
        }
        r
    }

    /// A signed request's answer. A sign-in Simkl no longer takes is renewed once and the
    /// request made again, as Simkl asks; a renewal it refuses means a new sign-in next time,
    /// and one that merely failed is tried again on the next refresh.
    fn get_json<T: serde::de::DeserializeOwned>(&mut self, path: &str, query: &str) -> Result<T, String> {
        let mut r = sdk::fetch_online("Simkl", &self.request(path, query, true))?;
        if r.status == 401 && error_of(&r.body).error == "invalid_token" {
            match renew(&self.client_id, &self.token, host::now_ms()) {
                Ok(fresh) => {
                    self.token = fresh;
                    r = sdk::fetch_online("Simkl", &self.request(path, query, true))?;
                }
                Err(Renew::Refused) => {
                    forget_account();
                    return Err(SIGN_IN_AGAIN.into());
                }
                Err(Renew::Failed(e)) => {
                    sdk::warn!("could not renew the sign-in ({e})");
                    self.token.expire();
                    return Err(RENEW_LATER.into());
                }
            }
        }
        if r.status == 401 {
            // Simkl no longer takes the token for some other reason; the next refresh renews it.
            sdk::warn!("HTTP 401 for {path}: {}", error_of(&r.body).error);
            self.token.expire();
            return Err(RENEW_LATER.into());
        }
        read_json(r, path)
    }

    /// A catalogue answer, which Simkl serves from its cache only when no sign-in is sent.
    fn get_public<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let r = sdk::fetch_online("Simkl", &self.request(path, "", false))?;
        read_json(r, path)
    }

    /// One list, newest first, cut to what the key-value store holds comfortably.
    fn list(&mut self, bucket: Bucket) -> Result<Vec<Title>, String> {
        let page: ListPage = self.get_json(&format!("/sync/all-items/{}/{}", bucket.kind.api(), bucket.status.api()), "&next_watch_info=yes")?;
        let rows = page.shows.or(page.anime).or(page.movies).unwrap_or_default();
        let mut titles: Vec<Title> = rows.iter().filter_map(|r| Title::from_row(r, bucket)).collect();
        match bucket.status {
            Status::Watching => titles.sort_by_key(|t| std::cmp::Reverse(t.last_watched_ms.unwrap_or(0))),
            Status::PlanToWatch => titles.sort_by_key(|t| std::cmp::Reverse(t.added_ms.unwrap_or(0))),
        }
        titles.truncate(bucket.status.kept());
        Ok(titles)
    }
}

fn app_query(client_id: &str) -> String {
    format!("client_id={}&app-name={APP_NAME}&app-version={}", uri_encode(client_id), env!("CARGO_PKG_VERSION"))
}

fn user_agent() -> String {
    format!("{APP_NAME}/{}", env!("CARGO_PKG_VERSION"))
}

fn read_json<T: serde::de::DeserializeOwned>(r: sdk::HttpResponse, path: &str) -> Result<T, String> {
    match r.status {
        412 => return Err(UNKNOWN_CLIENT.into()),
        429 => {
            let e = error_of(&r.body);
            // The day's allowance, as opposed to the second's: nothing more is asked until it
            // comes back, and what is stored serves meanwhile.
            if matches!(e.error.as_str(), "user_limit_exceeded" | "app_limit_exceeded") {
                let wait_ms = r.headers.get("retry-after").and_then(|v| v.trim().parse::<i64>().ok()).map_or(HOUR_MS, |s| s * 1000).clamp(5 * 60_000, DAY_MS);
                host::kv_set(PAUSE_KEY, (host::now_ms() + wait_ms).to_string().as_bytes());
                return Err("Simkl's daily allowance for this account is used up. The rows stay as they are until it comes back.".into());
            }
            return Err(PAUSE.into());
        }
        s if !(200..300).contains(&s) => {
            sdk::warn!("HTTP {s} for {path}: {}", error_of(&r.body).error);
            return Err(sdk::online_error("Simkl", s));
        }
        _ => {}
    }
    serde_json::from_str(&r.body).map_err(|e| {
        sdk::warn!("unexpected answer for {path}: {e}");
        sdk::unreadable("Simkl", API)
    })
}

/// A form post to the sign-in endpoints, which take no sign-in themselves.
fn post_form(path: &str, client_id: &str, fields: &[(&str, &str)]) -> Result<(u16, String), String> {
    let body = fields.iter().map(|(k, v)| format!("{k}={}", uri_encode(v))).collect::<Vec<_>>().join("&");
    let req = HttpRequest { method: "POST".into(), url: format!("{API}{path}?{}", app_query(client_id)), headers: Default::default(), body: Some(body), timeout_ms: FETCH_TIMEOUT_MS }
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("User-Agent", &user_agent());
    let r = sdk::fetch_online("Simkl", &req)?;
    Ok((r.status, r.body))
}

/// The token once the sign-in has gone through, so the refresh goes on with it, or else the
/// sentence the plugin's page shows meanwhile. One poll per refresh is all the host's rhythm
/// allows, and well over the five seconds Simkl asks between polls.
fn device_step(client_id: &str, now_ms: i64) -> Result<Token, String> {
    let pending: Option<Device> = host::kv_get(DEVICE_KEY).and_then(|v| serde_json::from_slice(&v).ok()).filter(|d: &Device| d.expires_ms > now_ms);
    if let Some(d) = pending {
        let (status, body) = post_form("/oauth2/token", client_id, &[("grant_type", "urn:ietf:params:oauth:grant-type:device_code"), ("client_id", client_id), ("device_code", &d.device_code)])?;
        if (200..300).contains(&status) {
            let t: TokenAnswer = serde_json::from_str(&body).map_err(unreadable_sign_in)?;
            // A new sign-in may be another account, so nothing of the last one is kept.
            forget_account();
            return Ok(save_token(&t, None, client_id, now_ms));
        }
        let e = error_of(&body);
        match e.error.as_str() {
            "expired_token" | "invalid_grant" | "access_denied" => {
                sdk::info!("sign-in code is no longer good ({}); asking for a new one", e.error);
                host::kv_delete(DEVICE_KEY);
            }
            "invalid_client" => return Err(NOT_A_TV_APP.into()),
            // Pending, asked to slow down, or an answer that says nothing: the code stands.
            other => {
                if !matches!(other, "authorization_pending" | "slow_down") {
                    sdk::info!("HTTP {status} while waiting for the sign-in ({other}); keeping the code");
                }
                return Err(instruction(&d));
            }
        }
    }
    let (status, body) = post_form("/oauth2/device", client_id, &[("client_id", client_id), ("scope", "media:read")])?;
    if !(200..300).contains(&status) {
        let e = error_of(&body);
        return Err(match (status, e.error.as_str()) {
            (401, _) | (_, "invalid_client") => NOT_A_TV_APP.into(),
            (412, _) | (_, "client_id_failed") => UNKNOWN_CLIENT.into(),
            (429, _) => PAUSE.into(),
            _ => {
                sdk::warn!("HTTP {status} when asked for a sign-in code: {} {}", e.error, e.error_description.or(e.message).unwrap_or_default());
                sdk::online_error("Simkl", status)
            }
        });
    }
    let a: DeviceAnswer = serde_json::from_str(&body).map_err(unreadable_sign_in)?;
    let d = Device {
        device_code: a.device_code,
        user_code: a.user_code,
        verification_uri: a.verification_uri.or(a.verification_url).unwrap_or_else(|| "https://simkl.com/pin".into()),
        expires_ms: now_ms + a.expires_in * 1000,
    };
    host::kv_set(DEVICE_KEY, serde_json::to_string(&d).unwrap_or_default().as_bytes());
    Err(instruction(&d))
}

fn instruction(d: &Device) -> String {
    sdk::sign_in(&d.verification_uri, &d.user_code)
}

/// The error for a sign-in answer that cannot be read; the cause goes to the log.
fn unreadable_sign_in(e: serde_json::Error) -> String {
    sdk::warn!("unexpected sign-in answer: {e}");
    sdk::unreadable("Simkl", API)
}

enum Renew {
    /// Simkl refused the refresh token, or there is none: it has run out or been revoked.
    Refused,
    Failed(String),
}

fn renew(client_id: &str, old: &Token, now_ms: i64) -> Result<Token, Renew> {
    if old.refresh.is_empty() {
        return Err(Renew::Refused);
    }
    let (status, body) = post_form("/oauth2/token", client_id, &[("grant_type", "refresh_token"), ("client_id", client_id), ("refresh_token", &old.refresh)]).map_err(Renew::Failed)?;
    if (200..300).contains(&status) {
        let t: TokenAnswer = serde_json::from_str(&body).map_err(|e| Renew::Failed(e.to_string()))?;
        return Ok(save_token(&t, Some(&old.refresh), client_id, now_ms));
    }
    let e = error_of(&body);
    match (status, e.error.as_str()) {
        (400, "invalid_grant") | (401, _) => {
            sdk::warn!("Simkl refused to renew the sign-in ({}); signing in again", e.error);
            Err(Renew::Refused)
        }
        _ => Err(Renew::Failed(format!("HTTP {status} {}", e.error))),
    }
}

/// Simkl repeats the refresh token on every renewal, so the one it gave first is kept.
fn save_token(t: &TokenAnswer, refresh: Option<&str>, client_id: &str, now_ms: i64) -> Token {
    let token = Token {
        access: t.access_token.clone(),
        refresh: t.refresh_token.clone().filter(|r| !r.is_empty()).or_else(|| refresh.map(str::to_string)).unwrap_or_default(),
        expires_ms: now_ms + t.expires_in.unwrap_or(WEEK_MS / 1000) * 1000,
        client_id: client_id.to_string(),
    };
    token.save();
    token
}

// ---------------------------------------------------------------------------
// What Simkl keeps lists of, and when each last changed
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
pub struct Activities {
    #[serde(default)]
    pub all: Option<String>,
    #[serde(default)]
    pub tv_shows: Domain,
    #[serde(default)]
    pub anime: Domain,
    #[serde(default)]
    pub movies: Domain,
}

#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
pub struct Domain {
    #[serde(default)]
    pub all: Option<String>,
    #[serde(default)]
    pub watching: Option<String>,
    #[serde(default)]
    pub plantowatch: Option<String>,
    #[serde(default)]
    pub playback: Option<String>,
    #[serde(default)]
    pub removed_from_list: Option<String>,
}

impl Domain {
    fn playback_stamps(a: &Activities) -> [Option<&String>; 3] {
        [a.tv_shows.playback.as_ref(), a.anime.playback.as_ref(), a.movies.playback.as_ref()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TitleKind {
    Show,
    Anime,
    Movie,
}

impl TitleKind {
    fn api(self) -> &'static str {
        match self {
            TitleKind::Show => "shows",
            TitleKind::Anime => "anime",
            TitleKind::Movie => "movies",
        }
    }

    /// The catalogue path for one title's summary.
    fn summary(self) -> &'static str {
        match self {
            TitleKind::Show => "tv",
            TitleKind::Anime => "anime",
            TitleKind::Movie => "movies",
        }
    }

    fn domain(self, a: &Activities) -> &Domain {
        match self {
            TitleKind::Show => &a.tv_shows,
            TitleKind::Anime => &a.anime,
            TitleKind::Movie => &a.movies,
        }
    }

    fn removed(self, a: &Activities) -> Option<&String> {
        self.domain(a).removed_from_list.as_ref()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Status {
    Watching,
    PlanToWatch,
}

impl Status {
    fn api(self) -> &'static str {
        match self {
            Status::Watching => "watching",
            Status::PlanToWatch => "plantowatch",
        }
    }

    fn kept(self) -> usize {
        match self {
            Status::Watching => KEPT_WATCHING,
            Status::PlanToWatch => KEPT_PLANNED,
        }
    }
}

/// One of Simkl's lists: a kind of title in a status.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bucket {
    pub kind: TitleKind,
    pub status: Status,
}

impl Bucket {
    const ALL: [Bucket; 5] = [
        Bucket { kind: TitleKind::Show, status: Status::Watching },
        Bucket { kind: TitleKind::Anime, status: Status::Watching },
        Bucket { kind: TitleKind::Show, status: Status::PlanToWatch },
        Bucket { kind: TitleKind::Anime, status: Status::PlanToWatch },
        Bucket { kind: TitleKind::Movie, status: Status::PlanToWatch },
    ];

    fn key(self) -> String {
        format!("list:{}:{}", self.kind.api(), self.status.api())
    }

    /// When Simkl says this list last changed.
    fn stamp(self, a: &Activities) -> Option<&String> {
        let d = self.kind.domain(a);
        match self.status {
            Status::Watching => d.watching.as_ref(),
            Status::PlanToWatch => d.plantowatch.as_ref(),
        }
    }

    /// Gives the list a stamp Simkl never does, so it counts as changed whatever comes next.
    fn forget_stamp(self, a: &mut Activities) {
        let d = match self.kind {
            TitleKind::Show => &mut a.tv_shows,
            TitleKind::Anime => &mut a.anime,
            TitleKind::Movie => &mut a.movies,
        };
        let stamp = match self.status {
            Status::Watching => &mut d.watching,
            Status::PlanToWatch => &mut d.plantowatch,
        };
        *stamp = Some(String::new());
    }
}

/// The lists as read, by bucket.
#[derive(Default)]
struct Lists {
    by_bucket: Vec<(Bucket, Vec<Title>)>,
}

impl Lists {
    fn put(&mut self, bucket: Bucket, titles: Vec<Title>) {
        self.by_bucket.push((bucket, titles));
    }

    fn all(&self) -> impl Iterator<Item = &Title> {
        self.by_bucket.iter().flat_map(|(_, t)| t.iter())
    }

    fn watching(&self) -> impl Iterator<Item = &Title> {
        self.by_bucket.iter().filter(|(b, _)| b.status == Status::Watching).flat_map(|(_, t)| t.iter())
    }

    fn planned(&self) -> impl Iterator<Item = &Title> {
        self.by_bucket.iter().filter(|(b, _)| b.status == Status::PlanToWatch).flat_map(|(_, t)| t.iter())
    }

    fn find(&self, id: u64) -> Option<&Title> {
        self.all().find(|t| t.id == id)
    }
}

// ---------------------------------------------------------------------------
// What Simkl sends
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Ids {
    #[serde(default)]
    pub simkl: Option<u64>,
    #[serde(default)]
    pub simkl_id: Option<u64>,
}

impl Ids {
    fn id(&self) -> Option<u64> {
        self.simkl.or(self.simkl_id).filter(|&id| id > 0)
    }
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Media {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub year: Option<u32>,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub fanart: Option<String>,
    /// Minutes.
    #[serde(default)]
    pub runtime: Option<u32>,
    #[serde(default)]
    pub ids: Ids,
    #[serde(default)]
    pub anime_type: Option<String>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct NextInfo {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub season: Option<u32>,
    #[serde(default)]
    pub episode: Option<u32>,
    #[serde(default)]
    pub date: Option<String>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Row {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub last_watched_at: Option<String>,
    #[serde(default)]
    pub added_to_watchlist_at: Option<String>,
    #[serde(default)]
    pub next_to_watch: Option<String>,
    #[serde(default)]
    pub next_to_watch_info: Option<NextInfo>,
    #[serde(default)]
    pub watched_episodes_count: Option<u32>,
    #[serde(default)]
    pub total_episodes_count: Option<u32>,
    #[serde(default)]
    pub show: Option<Media>,
    #[serde(default)]
    pub movie: Option<Media>,
    #[serde(default)]
    pub anime_type: Option<String>,
}

#[derive(Deserialize, Default)]
struct ListPage {
    #[serde(default)]
    shows: Option<Vec<Row>>,
    #[serde(default)]
    anime: Option<Vec<Row>>,
    #[serde(default)]
    movies: Option<Vec<Row>>,
}

/// The next episode of a show, as Simkl works it out. An anime counts its episodes straight
/// through and has no season.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Next {
    #[serde(default)]
    pub season: Option<u32>,
    pub episode: u32,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub date_ms: Option<i64>,
}

/// "S02E04" as Simkl writes the next episode, or "E12" for an anime.
pub fn parse_next(code: &str) -> Option<(Option<u32>, u32)> {
    let code = code.trim().to_ascii_uppercase();
    let (season, episode) = code.split_once('E')?;
    let season = season.trim_start_matches('S');
    let season = if season.is_empty() { None } else { Some(season.parse().ok()?) };
    Some((season, episode.parse().ok()?))
}

/// A title on one of the lists, as kept between refreshes.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Title {
    pub id: u64,
    pub kind: TitleKind,
    pub title: String,
    #[serde(default)]
    pub year: Option<u32>,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub runtime: Option<u32>,
    #[serde(default)]
    pub anime_type: Option<String>,
    #[serde(default)]
    pub next: Option<Next>,
    #[serde(default)]
    pub last_watched_ms: Option<i64>,
    #[serde(default)]
    pub added_ms: Option<i64>,
}

impl Title {
    pub fn from_row(r: &Row, bucket: Bucket) -> Option<Title> {
        let m = match bucket.kind {
            TitleKind::Movie => r.movie.as_ref()?,
            _ => r.show.as_ref()?,
        };
        let id = m.ids.id()?;
        if m.title.trim().is_empty() {
            return None;
        }
        let next = match (bucket.kind, &r.next_to_watch, &r.next_to_watch_info) {
            (TitleKind::Movie, ..) | (_, None, _) => None,
            (kind, Some(code), info) => {
                let (season, episode) = match info.as_ref().and_then(|i| Some((i.season, i.episode?))) {
                    Some(pair) => pair,
                    None => parse_next(code)?,
                };
                Some(Next {
                    season: season.filter(|_| kind != TitleKind::Anime),
                    episode,
                    title: episode_title(info.as_ref().and_then(|i| i.title.as_deref())),
                    date_ms: info.as_ref().and_then(|i| i.date.as_deref()).and_then(parse_iso8601),
                })
            }
        };
        Some(Title {
            id,
            kind: bucket.kind,
            title: m.title.trim().to_string(),
            year: m.year,
            poster: m.poster.clone().filter(|p| !p.is_empty()),
            runtime: m.runtime,
            anime_type: r.anime_type.clone().or_else(|| m.anime_type.clone()),
            next,
            last_watched_ms: r.last_watched_at.as_deref().and_then(parse_iso8601),
            added_ms: r.added_to_watchlist_at.as_deref().and_then(parse_iso8601),
        })
    }
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct PlaybackEpisode {
    #[serde(default)]
    pub season: u32,
    #[serde(default)]
    pub number: u32,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct PlaybackTitle {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub year: Option<u32>,
    #[serde(default)]
    pub ids: PlaybackIds,
    #[serde(default)]
    pub anime_type: Option<String>,
}

/// Ids kept between refreshes, so they serialise as they came.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct PlaybackIds {
    #[serde(default)]
    pub simkl: Option<u64>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Playback {
    #[serde(default)]
    pub progress: f32,
    #[serde(default)]
    pub paused_at: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub movie: Option<PlaybackTitle>,
    #[serde(default)]
    pub show: Option<PlaybackTitle>,
    #[serde(default)]
    pub anime: Option<PlaybackTitle>,
    #[serde(default)]
    pub episode: Option<PlaybackEpisode>,
}

impl Playback {
    fn title(&self) -> Option<&PlaybackTitle> {
        self.movie.as_ref().or(self.show.as_ref()).or(self.anime.as_ref()).filter(|t| !t.title.trim().is_empty())
    }

    fn title_kind(&self) -> TitleKind {
        if self.movie.is_some() {
            TitleKind::Movie
        } else if self.anime.is_some() {
            TitleKind::Anime
        } else {
            TitleKind::Show
        }
    }
}

/// A title's summary from the catalogue: the artwork the lists leave out.
#[derive(Deserialize, Default)]
struct Summary {
    #[serde(default)]
    poster: Option<String>,
    #[serde(default)]
    fanart: Option<String>,
    #[serde(default)]
    runtime: Option<u32>,
}

#[derive(Deserialize, Default, Debug)]
pub struct Found {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub year: Option<u32>,
    #[serde(default)]
    pub endpoint_type: Option<String>,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub ids: Ids,
}

// ---------------------------------------------------------------------------
// Artwork, remembered per title
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Art {
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub fanart: Option<String>,
    #[serde(default)]
    pub runtime: Option<u32>,
    pub fetched_ms: i64,
}

struct ArtCache {
    map: BTreeMap<u64, Art>,
    now_ms: i64,
    lookups_left: usize,
}

impl ArtCache {
    fn load(now_ms: i64) -> ArtCache {
        let map = host::kv_get(ART_KEY).and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or_default();
        ArtCache { map, now_ms, lookups_left: ART_LOOKUPS }
    }

    /// The title's artwork, looked up once a month while lookups are left this refresh. A
    /// title the catalogue does not know is remembered as having none, rather than asked again.
    fn lookup(&mut self, id: u64, kind: TitleKind, client: &Client) -> Art {
        if let Some(a) = self.map.get(&id).filter(|a| self.now_ms - a.fetched_ms < ART_FRESH_MS) {
            return a.clone();
        }
        if self.lookups_left == 0 {
            return self.map.get(&id).cloned().unwrap_or_default();
        }
        self.lookups_left -= 1;
        match client.get_public::<serde_json::Value>(&format!("/{}/{id}", kind.summary())) {
            Ok(v) => {
                let s: Summary = serde_json::from_value(v).unwrap_or_default();
                let art = Art { poster: s.poster, fanart: s.fanart, runtime: s.runtime, fetched_ms: self.now_ms };
                self.map.insert(id, art.clone());
                art
            }
            Err(e) => {
                if e.contains("budget") {
                    self.lookups_left = 0;
                }
                sdk::info!("summary {id}: {e}");
                self.map.get(&id).cloned().unwrap_or_default()
            }
        }
    }

    fn save(mut self) {
        if self.map.len() > ART_KEPT {
            let mut by_age: Vec<(i64, u64)> = self.map.iter().map(|(id, a)| (a.fetched_ms, *id)).collect();
            by_age.sort_unstable();
            for (_, id) in by_age.iter().take(self.map.len() - ART_KEPT) {
                self.map.remove(id);
            }
        }
        host::kv_set(ART_KEY, &serde_json::to_vec(&self.map).unwrap_or_default());
    }
}

/// A poster path as Simkl gives it, at a size that suits a poster tile.
pub fn poster_url(path: &str) -> String {
    format!("{IMAGES}/posters/{path}_m.webp")
}

/// A fanart path at the size of a landscape tile.
pub fn fanart_url(path: &str) -> String {
    format!("{IMAGES}/fanart/{path}_mobile.webp")
}

// ---------------------------------------------------------------------------
// The calendar: what airs soon, from Simkl's public files
// ---------------------------------------------------------------------------

/// One airing of a followed title. An anime episode has no season.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Airing {
    pub id: u64,
    pub date_ms: i64,
    #[serde(default)]
    pub season: Option<u32>,
    pub episode: u32,
    #[serde(default)]
    pub title: Option<String>,
}

/// A followed title as the calendar describes it, kept once however many airings it has.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Followed {
    pub title: String,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub fanart: Option<String>,
    #[serde(default)]
    pub network: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Feed {
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    /// The set of followed titles the entries were filtered for, so a change refilters.
    #[serde(default)]
    pub followed: u64,
    #[serde(default)]
    pub entries: Vec<Airing>,
    #[serde(default)]
    pub shows: BTreeMap<u64, Followed>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Calendar {
    #[serde(default)]
    pub feeds: BTreeMap<String, Feed>,
}

#[derive(Deserialize)]
struct CalendarFile<'a> {
    #[serde(default)]
    calendar: Vec<CalendarEntry>,
    #[serde(borrow, default)]
    metadata: HashMap<String, &'a serde_json::value::RawValue>,
}

#[derive(Deserialize)]
struct CalendarEntry {
    simkl_id: u64,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    episode: Option<CalendarEpisode>,
}

#[derive(Deserialize, Default)]
struct CalendarEpisode {
    #[serde(default)]
    season: Option<u32>,
    #[serde(default)]
    episode: Option<u32>,
    #[serde(default)]
    title: Option<String>,
}

impl Calendar {
    fn load() -> Calendar {
        host::kv_get(CALENDAR_KEY).and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or_default()
    }

    /// Stores the calendar, dropping the farthest airings until it fits.
    fn save(&mut self) {
        loop {
            let bytes = serde_json::to_vec(self).unwrap_or_default();
            if bytes.len() <= KV_ROOM {
                host::kv_set(CALENDAR_KEY, &bytes);
                return;
            }
            let Some(feed) = self.feeds.values_mut().max_by_key(|f| f.entries.len()).filter(|f| !f.entries.is_empty()) else {
                host::kv_set(CALENDAR_KEY, &bytes);
                return;
            };
            feed.entries.truncate(feed.entries.len() * 3 / 4);
            feed.prune_shows();
        }
    }

    /// Reads one calendar file when it, or what is followed, has changed since last time.
    fn update(&mut self, kind: TitleKind, followed: &HashSet<u64>, now_ms: i64) -> Result<(), String> {
        let name = match kind {
            TitleKind::Anime => "anime",
            _ => "tv",
        };
        let feed = self.feeds.entry(name.to_string()).or_default();
        let followed_hash = followed_hash(followed);
        if followed.is_empty() {
            *feed = Feed { followed: followed_hash, ..Feed::default() };
            return Ok(());
        }
        let url = format!("{CALENDAR}/{name}.json?{}", app_query(&host::setting("client_id").unwrap_or_default()));
        let mut req = HttpRequest { timeout_ms: FETCH_TIMEOUT_MS, ..HttpRequest::get(url) }.header("User-Agent", &user_agent());
        if feed.followed == followed_hash {
            if let Some(e) = &feed.etag {
                req = req.header("If-None-Match", e);
            }
            if let Some(m) = &feed.last_modified {
                req = req.header("If-Modified-Since", m);
            }
        }
        let r = sdk::fetch_online("Simkl", &req)?;
        if r.status == 304 {
            return Ok(());
        }
        if !r.ok() {
            return Err(sdk::online_error("Simkl", r.status));
        }
        let (entries, shows) = parse_calendar(&r.body, followed, now_ms).map_err(|e| {
            sdk::warn!("calendar {name}: {e}");
            sdk::unreadable("Simkl", CALENDAR)
        })?;
        *feed = Feed { etag: r.headers.get("etag").cloned(), last_modified: r.headers.get("last-modified").cloned(), followed: followed_hash, entries, shows };
        feed.prune_shows();
        Ok(())
    }
}

impl Feed {
    fn prune_shows(&mut self) {
        let used: HashSet<u64> = self.entries.iter().map(|a| a.id).collect();
        self.shows.retain(|id, _| used.contains(id));
    }
}

fn followed_hash(followed: &HashSet<u64>) -> u64 {
    let mut ids: Vec<u64> = followed.iter().copied().collect();
    ids.sort_unstable();
    ids.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, id| (h ^ id).wrapping_mul(0x0100_0000_01b3))
}

/// The coming week's airings of followed titles in a calendar file, soonest first, with each
/// title's details once. The file covers a month, so only what the row can show is kept.
pub fn parse_calendar(json: &str, followed: &HashSet<u64>, now_ms: i64) -> Result<(Vec<Airing>, BTreeMap<u64, Followed>), String> {
    let file: CalendarFile = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let mut shows: BTreeMap<u64, Followed> = BTreeMap::new();
    let mut out = Vec::new();
    for e in file.calendar.into_iter().filter(|e| followed.contains(&e.simkl_id)) {
        let Some(date_ms) = e.date.as_deref().and_then(parse_iso8601) else { continue };
        if date_ms < now_ms - DAY_MS || date_ms > now_ms + 8 * DAY_MS {
            continue;
        }
        let ep = e.episode.unwrap_or_default();
        let Some(episode) = ep.episode else { continue };
        if let std::collections::btree_map::Entry::Vacant(slot) = shows.entry(e.simkl_id) {
            let meta: Followed = file.metadata.get(&e.simkl_id.to_string()).and_then(|raw| serde_json::from_str(raw.get()).ok()).unwrap_or_default();
            if meta.title.trim().is_empty() {
                continue;
            }
            slot.insert(Followed { title: meta.title.trim().to_string(), network: meta.network.filter(|n| !n.trim().is_empty()), ..meta });
        }
        out.push(Airing { id: e.simkl_id, date_ms, season: ep.season, episode, title: episode_title(ep.title.as_deref()) });
    }
    out.sort_by_key(|a| a.date_ms);
    out.truncate(KEPT_AIRINGS);
    let used: HashSet<u64> = out.iter().map(|a| a.id).collect();
    shows.retain(|id, _| used.contains(id));
    Ok((out, shows))
}

// ---------------------------------------------------------------------------
// Tiles
// ---------------------------------------------------------------------------

/// The TV's own search for a title: the intent its global search provider answers.
pub fn tv_search_action(title: &str) -> Action {
    Action { view: Some(format!("intent:#Intent;action=android.search.action.GLOBAL_SEARCH;S.query={};end", uri_encode(title))), ..Default::default() }
}

/// The season and episode a tile shows: both for a show, the episode alone for an anime, whose
/// number then goes among the subtitle's facts since the tile writes it only with a season.
fn numbering(kind: TitleKind, season: Option<u32>, episode: u32, facts: &mut Vec<String>) -> (Option<String>, Option<String>) {
    match (kind, season) {
        (TitleKind::Anime, _) | (_, None) => {
            facts.insert(0, format!("Episode {episode}"));
            (None, Some(episode.to_string()))
        }
        (_, Some(s)) => (Some(s.to_string()), Some(episode.to_string())),
    }
}

fn resume_item(p: &Playback, lists: &Lists, art: &mut ArtCache, client: &Client, open: &dyn Fn(&str) -> Action) -> Option<Item> {
    let t = p.title()?;
    let id = t.ids.simkl.filter(|&id| id > 0)?;
    let kind = p.title_kind();
    let listed = lists.find(id);
    let look = art.lookup(id, kind, client);
    let progress = (p.progress / 100.0).clamp(0.0, 1.0);
    let runtime = listed.and_then(|l| l.runtime).or(look.runtime).filter(|&r| r > 0);
    let duration_ms = runtime.map(|r| i64::from(r) * 60_000);
    let poster = listed.and_then(|l| l.poster.as_deref()).or(look.poster.as_deref()).map(poster_url);
    let mut facts: Vec<String> = Vec::new();
    let (item_id, season, episode) = match (&p.episode, p.kind.as_str()) {
        (Some(e), _) => {
            facts.extend(episode_title(e.title.as_deref()));
            let season = (kind != TitleKind::Anime).then_some(e.season);
            let (s, n) = numbering(kind, season, e.number, &mut facts);
            (format!("resume:{id}:{}:{}", e.season, e.number), s, n)
        }
        (None, "movie") => {
            facts.extend(t.year.map(|y| y.to_string()));
            (format!("resume:{id}"), None, None)
        }
        _ => return None,
    };
    Some(Item {
        id: item_id,
        kind: ItemKind::Resume,
        title: t.title.trim().to_string(),
        subtitle: (!facts.is_empty()).then(|| facts.join(", ")),
        season,
        episode,
        duration_ms,
        position_ms: duration_ms.map(|d| (d as f32 * progress) as i64),
        progress: Some(progress),
        last_used_ms: p.paused_at.as_deref().and_then(parse_iso8601),
        poster,
        thumbnail: look.fanart.as_deref().map(fanart_url),
        action: open(t.title.trim()),
        ..Default::default()
    })
}

pub fn up_next_item(t: &Title, next: &Next, art: Art, weight: i32, now_ms: i64, open: &dyn Fn(&str) -> Action) -> Item {
    let mut facts: Vec<String> = next.title.iter().cloned().collect();
    let (season, episode) = numbering(t.kind, next.season, next.episode, &mut facts);
    Item {
        id: format!("next:{}", t.id),
        kind: ItemKind::Program,
        title: t.title.clone(),
        subtitle: (!facts.is_empty()).then(|| facts.join(", ")),
        season,
        episode,
        added_ms: t.last_watched_ms,
        is_new: sdk::new_this_week(next.date_ms, now_ms),
        poster: t.poster.as_deref().or(art.poster.as_deref()).map(poster_url),
        thumbnail: art.fanart.as_deref().map(fanart_url),
        row: Some("upnext".into()),
        weight,
        action: open(&t.title),
        ..Default::default()
    }
}

fn title_facts(t: &Title) -> Vec<String> {
    let mut facts: Vec<String> = Vec::new();
    if let Some(y) = t.year {
        facts.push(y.to_string());
    }
    if t.kind == TitleKind::Anime {
        facts.push("Anime".into());
    }
    facts
}

pub fn planned_item(t: &Title, now_ms: i64, weight: i32, open: &dyn Fn(&str) -> Action) -> Item {
    let facts = title_facts(t);
    Item {
        id: format!("plan:{}", t.id),
        kind: ItemKind::Program,
        title: t.title.clone(),
        subtitle: (!facts.is_empty()).then(|| facts.join(", ")),
        added_ms: t.added_ms,
        is_new: sdk::new_this_week(t.added_ms, now_ms),
        poster: t.poster.as_deref().map(poster_url),
        row: Some("planned".into()),
        weight,
        action: open(&t.title),
        ..Default::default()
    }
}

/// A title from a stored list as a search result.
pub fn listed_item(t: &Title, open: &dyn Fn(&str) -> Action) -> Item {
    let facts = title_facts(t);
    Item {
        id: format!("found:{}", t.id),
        kind: ItemKind::Program,
        title: t.title.clone(),
        subtitle: (!facts.is_empty()).then(|| facts.join(", ")),
        poster: t.poster.as_deref().map(poster_url),
        action: open(&t.title),
        ..Default::default()
    }
}

pub fn airing_item(a: &Airing, show: &Followed, now_ms: i64, utc_offset_minutes: i32, weight: i32, open: &dyn Fn(&str) -> Action) -> Item {
    let mut facts = vec![day_word(a.date_ms, now_ms, utc_offset_minutes)];
    facts.extend(a.title.clone());
    facts.extend(show.network.clone());
    let kind = if a.season.is_some() { TitleKind::Show } else { TitleKind::Anime };
    let (season, episode) = numbering(kind, a.season, a.episode, &mut facts);
    if kind == TitleKind::Anime {
        // The day comes first, then the episode.
        facts.swap(0, 1);
    }
    Item {
        id: match a.season {
            Some(s) => format!("air:{}:{s}:{}", a.id, a.episode),
            None => format!("air:{}:{}", a.id, a.episode),
        },
        kind: ItemKind::Program,
        title: show.title.clone(),
        subtitle: Some(facts.join(", ")),
        season,
        episode,
        start_ms: Some(a.date_ms),
        poster: show.poster.as_deref().map(poster_url),
        thumbnail: show.fanart.as_deref().map(fanart_url),
        row: Some("airing".into()),
        weight,
        action: open(&show.title),
        ..Default::default()
    }
}

pub fn found_item(f: &Found, open: &dyn Fn(&str) -> Action) -> Option<Item> {
    let id = f.ids.id()?;
    if f.title.trim().is_empty() {
        return None;
    }
    let mut facts: Vec<String> = Vec::new();
    if let Some(y) = f.year {
        facts.push(y.to_string());
    }
    match f.endpoint_type.as_deref() {
        Some("anime") => facts.push("Anime".into()),
        Some("movies") => facts.push("Film".into()),
        _ => {}
    }
    Some(Item {
        id: format!("found:{id}"),
        kind: ItemKind::Program,
        title: f.title.trim().to_string(),
        subtitle: (!facts.is_empty()).then(|| facts.join(", ")),
        poster: f.poster.as_deref().map(poster_url),
        action: open(f.title.trim()),
        ..Default::default()
    })
}

/// An episode title worth a line: not a placeholder, and not the number the tile shows anyway.
pub fn episode_title(title: Option<&str>) -> Option<String> {
    let t = title?.trim();
    if t.is_empty() || matches!(t.to_ascii_uppercase().as_str(), "TBA" | "TBD" | "TBC") {
        return None;
    }
    let generic = t.strip_prefix("Episode ").is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()));
    (!generic).then(|| t.to_string())
}

/// "Today", "Tomorrow", "Yesterday" or the weekday, in the TV's own time; a week or more on,
/// "next" and the weekday.
pub fn day_word(at_ms: i64, now_ms: i64, utc_offset_minutes: i32) -> String {
    const NAMES: [&str; 7] = ["Thursday", "Friday", "Saturday", "Sunday", "Monday", "Tuesday", "Wednesday"];
    let day = |ms: i64| (ms + i64::from(utc_offset_minutes) * 60_000).div_euclid(DAY_MS);
    let name = NAMES[day(at_ms).rem_euclid(7) as usize];
    match day(at_ms) - day(now_ms) {
        0 => "Today".into(),
        1 => "Tomorrow".into(),
        -1 => "Yesterday".into(),
        2..=6 => name.into(),
        _ => format!("Next {name}"),
    }
}

/// Percent-encoding as `Intent.parseUri` and a query string undo it: no plus signs for spaces.
fn uri_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const WATCHING: &str = r#"{"shows":[
      {"added_to_watchlist_at":"2025-02-24T23:55:13Z","last_watched_at":"2026-11-02T20:10:00Z","status":"watching","last_watched":"S02E03",
       "next_to_watch":"S02E04","next_to_watch_info":{"title":"Honeydew","season":2,"episode":4,"date":"2026-11-05T02:00:00Z"},
       "watched_episodes_count":11,"total_episodes_count":18,"not_aired_episodes_count":0,
       "show":{"title":"The Bear","poster":"24/24273cee77f9d9f","year":2022,"ids":{"simkl":1390,"slug":"the-bear","imdb":"tt14452776","tmdb":"136315"}}},
      {"status":"watching","last_watched_at":"2026-10-01T00:00:00Z","next_to_watch":null,"watched_episodes_count":73,"total_episodes_count":73,
       "show":{"title":"Game of Thrones","poster":"57/5742576cd8f59fcb0","year":2011,"ids":{"simkl":17465}}},
      {"status":"watching","next_to_watch":"S01E02","show":{"title":"  ","ids":{"simkl":5}}}
    ]}"#;

    #[test]
    fn a_watching_list_keeps_the_next_episode() {
        let page: ListPage = serde_json::from_str(WATCHING).unwrap();
        let bucket = Bucket { kind: TitleKind::Show, status: Status::Watching };
        let titles: Vec<Title> = page.shows.unwrap().iter().filter_map(|r| Title::from_row(r, bucket)).collect();
        assert_eq!(titles.len(), 2);
        let bear = &titles[0];
        assert_eq!((bear.id, bear.title.as_str()), (1390, "The Bear"));
        let next = bear.next.as_ref().unwrap();
        assert_eq!((next.season, next.episode, next.title.as_deref()), (Some(2), 4, Some("Honeydew")));
        assert_eq!(next.date_ms, parse_iso8601("2026-11-05T02:00:00Z"));
        assert_eq!(bear.last_watched_ms, parse_iso8601("2026-11-02T20:10:00Z"));
        assert!(titles[1].next.is_none());
        // Without the detail, the code alone names the episode.
        assert_eq!(parse_next("S02E04"), Some((Some(2), 4)));
        assert_eq!(parse_next("E12"), Some((None, 12)));
        assert_eq!(parse_next("s3e10"), Some((Some(3), 10)));
        assert_eq!(parse_next("soon"), None);
        let open = |t: &str| Action::search(t);
        let now = parse_iso8601("2026-11-10T00:00:00Z").unwrap();
        let item = up_next_item(bear, next, Art { fanart: Some("65/651708e80d87bb93".into()), ..Default::default() }, 12, now, &open);
        assert_eq!(item.id, "next:1390");
        assert_eq!(item.subtitle.as_deref(), Some("Honeydew"));
        assert_eq!((item.season.as_deref(), item.episode.as_deref()), (Some("2"), Some("4")));
        assert!(item.is_new);
        assert_eq!(item.poster.as_deref(), Some("https://simkl.in/posters/24/24273cee77f9d9f_m.webp"));
        assert_eq!(item.thumbnail.as_deref(), Some("https://simkl.in/fanart/65/651708e80d87bb93_mobile.webp"));
        assert_eq!(item.action.search.as_deref(), Some("The Bear"));
    }

    #[test]
    fn anime_count_episodes_straight_through() {
        let anime: ListPage = serde_json::from_str(r#"{"anime":[{"added_to_watchlist_at":"2026-11-08T00:13:09Z","last_watched_at":"2026-11-09T00:13:09Z","status":"watching","next_to_watch":"E05",
            "next_to_watch_info":{"title":"Ballad of Fallen Angels","episode":5,"date":"1998-05-01T00:00:00Z"},
            "watched_episodes_count":4,"total_episodes_count":26,"show":{"title":"Cowboy Bebop","poster":"36/36842f1bceb6b39","year":1998,"ids":{"simkl":37089,"mal":"1"}},"anime_type":"tv"}]}"#).unwrap();
        let t = Title::from_row(&anime.anime.unwrap()[0], Bucket { kind: TitleKind::Anime, status: Status::Watching }).unwrap();
        assert_eq!(t.anime_type.as_deref(), Some("tv"));
        let next = t.next.as_ref().unwrap();
        assert_eq!((next.season, next.episode), (None, 5));
        let open = |t: &str| tv_search_action(t);
        let item = up_next_item(&t, next, Art::default(), 3, 0, &open);
        assert_eq!((item.season, item.episode.as_deref()), (None, Some("5")));
        assert_eq!(item.subtitle.as_deref(), Some("Episode 5, Ballad of Fallen Angels"));
        assert!(!item.is_new);
        // A season Simkl still sends for an anime is left out all the same.
        let with_season: ListPage = serde_json::from_str(r#"{"anime":[{"status":"watching","next_to_watch":"S01E1100","next_to_watch_info":{"season":1,"episode":1100},"show":{"title":"One Piece","ids":{"simkl":40}}}]}"#).unwrap();
        let op = Title::from_row(&with_season.anime.unwrap()[0], Bucket { kind: TitleKind::Anime, status: Status::Watching }).unwrap();
        assert_eq!(op.next.as_ref().map(|n| (n.season, n.episode)), Some((None, 1100)));
    }

    #[test]
    fn films_and_plans() {
        let now = parse_iso8601("2026-11-10T00:00:00Z").unwrap();
        let open = |t: &str| tv_search_action(t);
        let anime: ListPage = serde_json::from_str(r#"{"anime":[{"added_to_watchlist_at":"2026-11-08T00:13:09Z","status":"plantowatch","next_to_watch":"E01",
            "show":{"title":"Cowboy Bebop","poster":"36/36842f1bceb6b39","year":1998,"ids":{"simkl":37089}},"anime_type":"tv"}]}"#).unwrap();
        let t = Title::from_row(&anime.anime.unwrap()[0], Bucket { kind: TitleKind::Anime, status: Status::PlanToWatch }).unwrap();
        let item = planned_item(&t, now, 3, &open);
        assert_eq!(item.id, "plan:37089");
        assert_eq!(item.subtitle.as_deref(), Some("1998, Anime"));
        assert!(item.is_new);
        assert_eq!(item.action.view.as_deref(), Some("intent:#Intent;action=android.search.action.GLOBAL_SEARCH;S.query=Cowboy%20Bebop;end"));
        assert_eq!(listed_item(&t, &open).id, "found:37089");

        let movies: ListPage = serde_json::from_str(r#"{"movies":[{"added_to_watchlist_at":"2026-04-10T20:13:02Z","status":"plantowatch","watched_episodes_count":0,"total_episodes_count":0,
            "movie":{"title":"The Godfather","poster":"10/102447562f6b49ff3a","year":1972,"ids":{"simkl":53434,"imdb":"tt0068646"}}}]}"#).unwrap();
        let m = Title::from_row(&movies.movies.unwrap()[0], Bucket { kind: TitleKind::Movie, status: Status::PlanToWatch }).unwrap();
        assert_eq!((m.kind, m.next.is_none()), (TitleKind::Movie, true));
        assert!(!planned_item(&m, now, 1, &open).is_new);
        // An empty list may come as null.
        let empty: ListPage = serde_json::from_str(r#"{"movies":null}"#).unwrap();
        assert!(empty.movies.is_none());
    }

    #[test]
    fn paused_playback_becomes_resume_points() {
        let list: Vec<Playback> = serde_json::from_str(r#"[
          {"id":1,"progress":22.25,"paused_at":"2026-11-09T21:00:00Z","type":"movie","movie":{"title":"Charade","year":1963,"ids":{"simkl":100,"slug":"charade"}}},
          {"id":2,"progress":60,"paused_at":"2026-11-08T20:00:00Z","type":"episode","episode":{"season":2,"number":4,"title":"Honeydew"},"show":{"title":"The Bear","year":2022,"ids":{"simkl":1390}}},
          {"id":3,"progress":10,"paused_at":"2026-11-07T20:00:00Z","type":"episode","episode":{"season":1,"number":5,"title":"Episode 5","tvdb_season":1,"tvdb_number":5},"anime":{"title":"Cowboy Bebop","year":1998,"anime_type":"tv","ids":{"simkl":37089}}}
        ]"#).unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].title_kind(), TitleKind::Movie);
        assert_eq!(list[2].title_kind(), TitleKind::Anime);
        assert_eq!(list[1].episode.as_ref().map(|e| (e.season, e.number)), Some((2, 4)));
        // Kept between refreshes as it came.
        let again: Vec<Playback> = serde_json::from_slice(&serde_json::to_vec(&list).unwrap()).unwrap();
        assert_eq!(again[0].title().unwrap().ids.simkl, Some(100));
        let mut facts = Vec::new();
        assert_eq!(numbering(TitleKind::Anime, Some(1), 5, &mut facts), (None, Some("5".into())));
        assert_eq!(facts, ["Episode 5"]);
    }

    #[test]
    fn the_calendar_keeps_only_followed_titles_this_week() {
        let json = r#"{"calendar":[
          {"simkl_id":3437,"date":"2026-11-12T04:00:00Z","finale_type":null,"episode":{"season":15,"episode":1,"title":"Failure to Hard Launch","runtime":24,"url":"https://simkl.com/tv/3437/king-of-the-hill/season-15/episode-1/"}},
          {"simkl_id":3437,"date":"2026-11-13T04:00:00Z","episode":{"season":15,"episode":2,"title":"TBA","runtime":24}},
          {"simkl_id":3437,"date":"2026-12-19T04:00:00Z","episode":{"season":15,"episode":9,"title":"Far off"}},
          {"simkl_id":99,"date":"2026-11-12T04:00:00Z","episode":{"season":1,"episode":1,"title":"Not followed"}},
          {"simkl_id":3437,"date":null,"episode":{"season":15,"episode":3}},
          {"simkl_id":777,"date":"2026-11-14T15:00:00+09:00","episode":{"episode":1101,"title":"Episode 1101"}}
        ],"metadata":{"3437":{"title":"King of the Hill","poster":"12/12634613c2b41dc5a1","fanart":"23/23782f1e28e9ca2","network":"Hulu","ids":{"simkl_id":3437},"rank":1820},
                      "777":{"title":"One Piece","poster":"77/77","network":""},
                      "99":{"title":"Other"}}}"#;
        let followed: HashSet<u64> = [3437, 777].into_iter().collect();
        let now = parse_iso8601("2026-11-11T12:00:00Z").unwrap();
        let (airings, shows) = parse_calendar(json, &followed, now).unwrap();
        assert_eq!(airings.iter().map(|a| (a.id, a.season, a.episode)).collect::<Vec<_>>(), [(3437, Some(15), 1), (3437, Some(15), 2), (777, None, 1101)]);
        assert_eq!(airings[1].title, None);
        assert_eq!(shows.len(), 2);
        assert_eq!(shows[&3437].network.as_deref(), Some("Hulu"));
        assert_eq!(shows[&777].network, None);
        let open = |t: &str| Action::search(t);
        let item = airing_item(&airings[0], &shows[&3437], now, 0, 5, &open);
        assert_eq!(item.id, "air:3437:15:1");
        assert_eq!(item.subtitle.as_deref(), Some("Tomorrow, Failure to Hard Launch, Hulu"));
        assert_eq!(item.start_ms, parse_iso8601("2026-11-12T04:00:00Z"));
        assert_eq!(item.thumbnail.as_deref(), Some("https://simkl.in/fanart/23/23782f1e28e9ca2_mobile.webp"));
        let anime = airing_item(&airings[2], &shows[&777], now, 0, 1, &open);
        assert_eq!(anime.id, "air:777:1101");
        assert_eq!((anime.season, anime.episode.as_deref()), (None, Some("1101")));
        assert_eq!(anime.subtitle.as_deref(), Some("Saturday, Episode 1101"));
        // West of Greenwich the same moment is still today; two days off, the weekday is named.
        assert_eq!(day_word(airings[0].date_ms, now, -300), "Today");
        assert_eq!(day_word(airings[0].date_ms, now - DAY_MS, 0), "Thursday");
        assert_eq!(day_word(now, now, 0), "Today");
        assert_eq!(day_word(now - DAY_MS, now, 0), "Yesterday");
        assert_eq!(day_word(now + 7 * DAY_MS, now, 0), "Next Wednesday");
        assert_ne!(followed_hash(&followed), followed_hash(&HashSet::new()));
        // Fewer airings leave fewer titles to keep.
        let mut feed = Feed { entries: airings.clone(), shows: shows.clone(), ..Feed::default() };
        feed.entries.truncate(2);
        feed.prune_shows();
        assert_eq!(feed.shows.len(), 1);
    }

    #[test]
    fn activities_decide_what_is_read() {
        let a: Activities = serde_json::from_str(r#"{"all":"2026-11-09T10:00:00Z","settings":{"all":null},
            "tv_shows":{"all":"2026-11-09T10:00:00Z","rated_at":null,"playback":"2026-11-09T09:00:00Z","plantowatch":"2026-10-01T00:00:00Z","watching":"2026-11-09T10:00:00Z","completed":null,"hold":null,"dropped":null,"removed_from_list":null},
            "anime":{"all":null,"watching":null,"plantowatch":null,"playback":null},
            "movies":{"all":"2026-11-01T00:00:00Z","plantowatch":"2026-11-01T00:00:00Z","playback":null,"completed":null,"dropped":null},
            "custom_lists":{"lists":{"all":null}}}"#).unwrap();
        let shows_watching = Bucket { kind: TitleKind::Show, status: Status::Watching };
        assert_eq!(shows_watching.stamp(&a).map(String::as_str), Some("2026-11-09T10:00:00Z"));
        assert_eq!(Bucket { kind: TitleKind::Anime, status: Status::Watching }.stamp(&a), None);
        assert_eq!(Bucket { kind: TitleKind::Movie, status: Status::PlanToWatch }.stamp(&a).map(String::as_str), Some("2026-11-01T00:00:00Z"));
        let b: Activities = serde_json::from_slice(&serde_json::to_vec(&a).unwrap()).unwrap();
        assert_eq!(a, b);
        // A list skipped while it changed differs from any later stamp, even none.
        let mut skipped: Activities = serde_json::from_slice(&serde_json::to_vec(&a).unwrap()).unwrap();
        let anime_watching = Bucket { kind: TitleKind::Anime, status: Status::Watching };
        shows_watching.forget_stamp(&mut skipped);
        anime_watching.forget_stamp(&mut skipped);
        assert_ne!(shows_watching.stamp(&skipped), shows_watching.stamp(&a));
        assert_ne!(anime_watching.stamp(&skipped), anime_watching.stamp(&a));
        assert_eq!(Bucket { kind: TitleKind::Show, status: Status::PlanToWatch }.stamp(&skipped), Bucket { kind: TitleKind::Show, status: Status::PlanToWatch }.stamp(&a));
        assert_eq!(Domain::playback_stamps(&a)[0].map(String::as_str), Some("2026-11-09T09:00:00Z"));
        assert_eq!(shows_watching.key(), "list:shows:watching");
    }

    #[test]
    fn search_results_and_sign_in_words() {
        let found: Vec<Found> = serde_json::from_str(r#"[{"title":"John Wick","year":2014,"endpoint_type":"movies","poster":"30/3002370dbc564e5d8","ids":{"simkl_id":342994,"slug":"john-wick","tmdb":"245891"}},
            {"title":"Cowboy Bebop","title_romaji":"Cowboy Bebop","year":1998,"endpoint_type":"anime","type":"tv","poster":"36/36842f1bceb6b39","ids":{"simkl_id":37089}},
            {"title":"Breaking Bad","year":2008,"endpoint_type":"tv","poster":null,"ids":{"simkl_id":11121}}]"#).unwrap();
        let open = |t: &str| tv_search_action(t);
        let items: Vec<Item> = found.iter().filter_map(|f| found_item(f, &open)).collect();
        assert_eq!(items[0].subtitle.as_deref(), Some("2014, Film"));
        assert_eq!(items[1].subtitle.as_deref(), Some("1998, Anime"));
        assert_eq!(items[2].subtitle.as_deref(), Some("2008"));
        assert!(items[2].poster.is_none());
        assert!(items[0].action.view.is_some());
        let d = Device { device_code: "x".into(), user_code: "BDWP-HQPK".into(), verification_uri: "https://simkl.com/pin".into(), expires_ms: 0 };
        assert_eq!(instruction(&d), "Sign in: on your phone open simkl.com/pin and enter BDWP-HQPK, then press Refresh now here.");
        let e = error_of(r#"{"error":"invalid_token","code":401,"message":"Access token has expired"}"#);
        assert_eq!(e.error, "invalid_token");
        let token = |expires_ms| Token { access: "a".into(), refresh: "r".into(), expires_ms, client_id: String::new() };
        assert!(token(7 * DAY_MS).due(6 * DAY_MS + 1));
        assert!(!token(7 * DAY_MS).due(5 * DAY_MS));
        assert!(matches!(renew("cid", &Token { refresh: String::new(), ..token(0) }, 0), Err(Renew::Refused)));
        assert_eq!(app_query("a b"), format!("client_id=a%20b&app-name=lounge&app-version={}", env!("CARGO_PKG_VERSION")));
        assert_eq!(uri_encode("The Bear & co"), "The%20Bear%20%26%20co");
        assert_eq!(episode_title(Some("Honeydew")).as_deref(), Some("Honeydew"));
        for dull in [Some("TBA"), Some("tbd"), Some("Episode 96"), Some("  "), None] {
            assert_eq!(episode_title(dull), None, "{dull:?}");
        }
        assert_eq!(episode_title(Some("Episode One")).as_deref(), Some("Episode One"));
    }
}
