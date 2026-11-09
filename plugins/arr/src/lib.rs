//! Sonarr and Radarr: what is coming up for the shows and films they follow, what is downloading
//! now, and what has just arrived. The servers know titles, not players, so selecting a tile
//! searches for its title: Lounge's own search, which finds it in whichever app has the file,
//! or the TV's.

use lounge_plugin_sdk::{self as sdk, host, parse_iso8601, Action, HttpRequest, Item, ItemKind, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use serde::Deserialize;
use std::collections::BTreeMap;

pub struct Arr;

const DAY_MS: i64 = 86_400_000;
/// Most tiles in Coming up.
const UPCOMING_ITEMS: usize = 60;
/// Most tiles in each of Downloading and New.
const RECENT_ITEMS: usize = 40;

impl sdk::Plugin for Arr {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.arr".into(),
            name: "Sonarr and Radarr".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "What is coming up, downloading now and newly arrived, from your Sonarr and Radarr.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content],
            fetch_hosts: vec!["$sonarr".into(), "$radarr".into(), "*".into()],
            refresh_minutes: 30,
            rows: vec![
                RowSpec { id: "upcoming".into(), title: "Coming up on Sonarr and Radarr".into(), style: RowStyle::Landscape },
                RowSpec { id: "downloading".into(), title: "Downloading on Sonarr and Radarr".into(), style: RowStyle::Landscape },
                RowSpec { id: "recent".into(), title: "New on Sonarr and Radarr".into(), style: RowStyle::Poster },
            ],
            settings: vec![
                SettingSpec { key: "sonarr".into(), label: "Sonarr address".into(), kind: SettingKind::Url, required: false, help: Some("Like http://192.168.0.10:8989, or …:8989/sonarr if you gave it a URL base. Leave empty if you only run Radarr.".into()), default: None },
                SettingSpec { key: "sonarr_key".into(), label: "Sonarr API key".into(), kind: SettingKind::Password, required: false, help: Some("In Sonarr under Settings, General, Security. It is 32 characters, easiest pasted from the phone.".into()), default: None },
                SettingSpec { key: "radarr".into(), label: "Radarr address".into(), kind: SettingKind::Url, required: false, help: Some("Like http://192.168.0.10:7878, or …:7878/radarr if you gave it a URL base. Leave empty if you only run Sonarr.".into()), default: None },
                SettingSpec { key: "radarr_key".into(), label: "Radarr API key".into(), kind: SettingKind::Password, required: false, help: Some("In Radarr under Settings, General, Security. It is 32 characters, easiest pasted from the phone.".into()), default: None },
                SettingSpec { key: "days".into(), label: "Days ahead".into(), kind: SettingKind::Number, required: false, help: Some("How many days ahead Coming up looks, from 1 to 60. Yesterday is always shown too.".into()), default: Some("7".into()) },
                SettingSpec {
                    key: "tv_search".into(),
                    label: "Open with the TV's own search".into(),
                    kind: SettingKind::Toggle,
                    required: false,
                    help: Some("Off: Lounge searches your apps and plugins for the title, so a Jellyfin or Plex plugin that is on finds it. On: the TV's search, which on Google TV lists where it streams.".into()),
                    default: Some("false".into()),
                },
            ],
            homepage: None,
            self_signed_servers: false,
            for_apps: Vec::new(),
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let mut failures = Vec::new();
        let mut servers = Vec::new();
        for (name, url_key, key_key) in [("Sonarr", "sonarr", "sonarr_key"), ("Radarr", "radarr", "radarr_key")] {
            match Server::from_settings(name, url_key, key_key) {
                Ok(Some(server)) => servers.push(server),
                Ok(None) => {}
                Err(e) => failures.push(e),
            }
        }
        if servers.is_empty() && failures.is_empty() {
            return Err("Nothing to show yet: add a Sonarr or Radarr address and API key in the plugin's settings".into());
        }
        let days = host::setting("days").and_then(|v| v.trim().parse::<i64>().ok()).unwrap_or(7).clamp(1, 60);
        let clock = Clock { now_ms: ctx.now_ms, utc_offset_minutes: ctx.utc_offset_minutes };
        let window = Window::new(&clock, days);
        let tv_search = host::setting("tv_search").map(|v| v == "true").unwrap_or(false);
        let open = |title: &str| if tv_search { tv_search_action(title) } else { Action::search(title) };

        let mut upcoming: Vec<(i64, Item)> = Vec::new();
        let mut downloading: Vec<Item> = Vec::new();
        let mut recent: Vec<Item> = Vec::new();
        for s in &servers {
            if let Err(e) = s.check_identity() {
                failures.push(e);
                continue;
            }
            let outcome = if s.name == "Sonarr" { s.sonarr(&window, &clock, &open) } else { s.radarr(&window, &clock, &open) };
            match outcome {
                Ok(found) => {
                    upcoming.extend(found.upcoming);
                    downloading.extend(found.downloading);
                    recent.extend(found.recent);
                }
                Err(e) => failures.push(e),
            }
        }
        for e in &failures {
            sdk::warn!("{e}");
        }
        if upcoming.is_empty()
            && downloading.is_empty()
            && recent.is_empty()
            && let Some(e) = failures.into_iter().next()
        {
            return Err(e);
        }

        let mut items = Vec::new();
        upcoming.sort_by_key(|(at, _)| *at);
        upcoming.truncate(UPCOMING_ITEMS);
        let count = upcoming.len();
        items.extend(upcoming.into_iter().enumerate().map(|(i, (_, mut it))| {
            it.weight = (count - i) as i32;
            it
        }));
        downloading.truncate(RECENT_ITEMS);
        let active = downloading.len();
        items.extend(downloading.into_iter().enumerate().map(|(i, mut it)| {
            it.weight = (active - i) as i32;
            it
        }));
        recent.sort_by_key(|it| std::cmp::Reverse(it.added_ms));
        recent.truncate(RECENT_ITEMS);
        let arrived = recent.len();
        items.extend(recent.into_iter().enumerate().map(|(i, mut it)| {
            it.weight = (arrived - i) as i32;
            it.is_new = sdk::new_this_week(it.added_ms, ctx.now_ms);
            it
        }));
        sdk::dedup_ids(&mut items);
        sdk::info!("{count} coming up, {active} downloading, {arrived} newly arrived");
        Ok(items)
    }
}

sdk::export_plugin!(Arr);

// ---------------------------------------------------------------------------
// The servers
// ---------------------------------------------------------------------------

struct Server {
    name: &'static str,
    url: String,
    key: String,
}

/// What one server contributed: Coming up tiles with their moment, and the other two rows.
#[derive(Default)]
struct Found {
    upcoming: Vec<(i64, Item)>,
    downloading: Vec<Item>,
    recent: Vec<Item>,
}

#[derive(Deserialize, Default)]
struct Status {
    #[serde(rename = "appName", default)]
    app_name: String,
}

impl Server {
    /// The server the settings name, or none when its address is empty.
    fn from_settings(name: &'static str, url_key: &str, key_key: &str) -> Result<Option<Server>, String> {
        let Some(url) = host::setting(url_key).map(|u| sdk::server_address(&u)).filter(|u| u != "http://") else {
            return Ok(None);
        };
        let key = host::secret(key_key).map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).ok_or(format!("Add the {name} API key in the plugin's settings."))?;
        Ok(Some(Server { name, url, key }))
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let req = HttpRequest::get(format!("{}{path}", self.url)).header("X-Api-Key", &self.key).header("Accept", "application/json");
        let r = sdk::fetch_from(self.name, &self.url, &req).map_err(|e| {
            if e == host::TOO_LARGE {
                format!("{} sent more than this plugin can read in one go. Try again later.", self.name)
            } else if e.starts_with("fetch budget") {
                "Too much was asked for this refresh; the rest comes with the next.".to_string()
            } else {
                e
            }
        })?;
        let what = path.split('?').next().unwrap_or(path);
        match r.status {
            200..=299 => serde_json::from_str(&r.body).map_err(|e| {
                sdk::warn!("unexpected answer for {what}: {e}");
                sdk::unreadable(self.name, &self.url)
            }),
            401 => Err(format!("{} rejected the API key. Check it in the plugin's settings.", self.name)),
            404 => Err(sdk::wrong_server(self.name, &self.url)),
            s => {
                sdk::warn!("HTTP {s} for {what}");
                Err(sdk::answered_error(self.name, &self.url, s))
            }
        }
    }

    /// Makes sure the address answers as the server it is meant to be, so swapped addresses
    /// are said rather than quietly showing nothing.
    fn check_identity(&self) -> Result<(), String> {
        let status: Status = self.get_json("/api/v3/system/status")?;
        if status.app_name.is_empty() || status.app_name.eq_ignore_ascii_case(self.name) {
            return Ok(());
        }
        Err(format!("The {} address answers as {}. Check the addresses in the plugin's settings.", self.name, status.app_name))
    }

    fn sonarr(&self, window: &Window, clock: &Clock, open: &dyn Fn(&str) -> Action) -> Result<Found, String> {
        let episodes: Vec<Episode> = self.get_json(&format!("/api/v3/calendar?start={}&end={}&includeSeries=true&includeEpisodeImages=true", iso_utc(window.start_ms), iso_utc(window.end_ms)))?;
        let mut found = Found { upcoming: upcoming_episodes(episodes, window, clock, open), ..Default::default() };
        match self.get_json::<QueuePage<QueueRecord>>("/api/v3/queue?page=1&pageSize=50&includeSeries=true&includeEpisode=true") {
            Ok(page) => found.downloading = downloading_items(page.records, open),
            Err(e) => sdk::warn!("Sonarr queue: {e}"),
        }
        match self.get_json::<HistoryPage<EpisodeHistory>>("/api/v3/history?page=1&pageSize=60&sortKey=date&sortDirection=descending&eventType=3&includeSeries=true&includeEpisode=true") {
            Ok(page) => found.recent = arrived_episodes(page.records, open),
            Err(e) => sdk::warn!("Sonarr history: {e}"),
        }
        Ok(found)
    }

    fn radarr(&self, window: &Window, clock: &Clock, open: &dyn Fn(&str) -> Action) -> Result<Found, String> {
        // Radarr's dates are days; a day's edge in the TV's time zone may fall outside the day in UTC.
        let movies: Vec<Movie> = self.get_json(&format!("/api/v3/calendar?start={}&end={}", iso_utc(window.start_ms - DAY_MS), iso_utc(window.end_ms + DAY_MS)))?;
        let mut found = Found { upcoming: upcoming_movies(movies, window, clock, open), ..Default::default() };
        match self.get_json::<QueuePage<QueueRecord>>("/api/v3/queue?page=1&pageSize=50&includeMovie=true") {
            Ok(page) => found.downloading = downloading_items(page.records, open),
            Err(e) => sdk::warn!("Radarr queue: {e}"),
        }
        match self.get_json::<HistoryPage<MovieHistory>>("/api/v3/history?page=1&pageSize=40&sortKey=date&sortDirection=descending&eventType=3&includeMovie=true") {
            Ok(page) => found.recent = arrived_movies(page.records, open),
            Err(e) => sdk::warn!("Radarr history: {e}"),
        }
        Ok(found)
    }
}

// ---------------------------------------------------------------------------
// What they send
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Image {
    #[serde(rename = "coverType", default)]
    pub cover_type: String,
    #[serde(rename = "remoteUrl", default)]
    pub remote_url: Option<String>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Series {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub year: Option<i64>,
    #[serde(default)]
    pub images: Vec<Image>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Episode {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub title: String,
    #[serde(rename = "seasonNumber", default)]
    pub season: i64,
    #[serde(rename = "episodeNumber", default)]
    pub episode: i64,
    #[serde(rename = "airDateUtc", default)]
    pub air_date_utc: Option<String>,
    #[serde(rename = "hasFile", default)]
    pub has_file: bool,
    #[serde(rename = "finaleType", default)]
    pub finale_type: Option<String>,
    #[serde(default)]
    pub series: Option<Series>,
    #[serde(default)]
    pub images: Vec<Image>,
}

impl Episode {
    /// Sonarr's placeholder for an episode not yet named.
    fn named_title(&self) -> Option<String> {
        Some(self.title.trim().to_string()).filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case("TBA"))
    }

    /// What makes the episode an occasion, as Sonarr's own calendar marks it.
    fn occasion(&self) -> Option<&'static str> {
        match self.finale_type.as_deref() {
            Some("series") => return Some("Series finale"),
            Some("season") => return Some("Season finale"),
            Some("midseason") => return Some("Midseason finale"),
            _ => {}
        }
        match (self.season, self.episode) {
            (1, 1) => Some("Series premiere"),
            (s, 1) if s > 1 => Some("Season premiere"),
            _ => None,
        }
    }
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Movie {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub year: Option<i64>,
    #[serde(default)]
    pub images: Vec<Image>,
    #[serde(rename = "inCinemas", default)]
    pub in_cinemas: Option<String>,
    #[serde(rename = "digitalRelease", default)]
    pub digital_release: Option<String>,
    #[serde(rename = "physicalRelease", default)]
    pub physical_release: Option<String>,
    #[serde(rename = "hasFile", default)]
    pub has_file: bool,
}

#[derive(Deserialize, Default)]
pub struct HistoryPage<T> {
    #[serde(default)]
    pub records: Vec<T>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct EpisodeHistory {
    #[serde(rename = "episodeId", default)]
    pub episode_id: i64,
    #[serde(rename = "seriesId", default)]
    pub series_id: i64,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub episode: Option<Episode>,
    #[serde(default)]
    pub series: Option<Series>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct MovieHistory {
    #[serde(rename = "movieId", default)]
    pub movie_id: i64,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub movie: Option<Movie>,
}

#[derive(Deserialize, Default)]
pub struct QueuePage<T> {
    #[serde(default)]
    pub records: Vec<T>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct QueueRecord {
    #[serde(rename = "downloadId", default)]
    pub download_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(rename = "trackedDownloadState", default)]
    pub tracked_state: Option<String>,
    #[serde(default)]
    pub series: Option<Series>,
    #[serde(default)]
    pub episode: Option<Episode>,
    #[serde(default)]
    pub movie: Option<Movie>,
}

impl QueueRecord {
    /// One word for where the download is, in the viewer's terms.
    fn state(&self) -> &'static str {
        match (self.tracked_state.as_deref().unwrap_or(""), self.status.as_deref().unwrap_or("")) {
            (s, _) if s.starts_with("import") => "Arriving",
            (_, "paused") => "Paused",
            (_, "queued") | (_, "delay") => "Waiting",
            (_, "completed") => "Arriving",
            (_, "failed") | (_, "warning") => "Stuck",
            _ => "Downloading",
        }
    }
}

/// A picture of one kind from the server's list, by its public address, so the TV fetches it
/// without the API key. TMDB's originals are asked for in a size a TV needs.
pub fn picture(images: &[Image], cover_type: &str) -> Option<String> {
    let url = images.iter().find(|i| i.cover_type.eq_ignore_ascii_case(cover_type)).and_then(|i| i.remote_url.clone()).map(|u| u.trim().to_string()).filter(|u| u.starts_with("https://") || u.starts_with("http://"))?;
    Some(match url.strip_prefix("https://image.tmdb.org/t/p/original/") {
        Some(rest) if cover_type.eq_ignore_ascii_case("poster") => format!("https://image.tmdb.org/t/p/w500/{rest}"),
        Some(rest) => format!("https://image.tmdb.org/t/p/w1280/{rest}"),
        None => url,
    })
}

// ---------------------------------------------------------------------------
// Days, in the TV's own time
// ---------------------------------------------------------------------------

/// The TV's own now and day, for saying when something is on.
pub struct Clock {
    pub now_ms: i64,
    pub utc_offset_minutes: i32,
}

impl Clock {
    fn offset_ms(&self) -> i64 {
        self.utc_offset_minutes as i64 * 60_000
    }

    /// The TV's day a moment falls in, counted from 1970-01-01.
    pub fn day_of(&self, ms: i64) -> i64 {
        (ms + self.offset_ms()).div_euclid(DAY_MS)
    }

    /// The day a calendar date stands for. Servers store a date as midnight, UTC or their own,
    /// so the nearest midnight is the date meant.
    pub fn day_of_date(&self, ms: i64) -> i64 {
        (ms + DAY_MS / 2).div_euclid(DAY_MS)
    }

    pub fn today(&self) -> i64 {
        self.day_of(self.now_ms)
    }

    /// The first moment of a day, in the TV's time.
    fn start_of(&self, day: i64) -> i64 {
        day * DAY_MS - self.offset_ms()
    }

    /// When a day falls, as a person would say it: Today, Tomorrow, a weekday within the week,
    /// Yesterday, or the date.
    pub fn when(&self, day: i64) -> String {
        match day - self.today() {
            -1 => "Yesterday".into(),
            0 => "Today".into(),
            1 => "Tomorrow".into(),
            2..=6 => weekday(day).into(),
            _ => {
                let (_, month, d) = civil(day);
                format!("{d} {}", MONTHS[month as usize - 1])
            }
        }
    }
}

/// The days Coming up covers: yesterday through `days` ahead, whole days in the TV's time.
struct Window {
    first_day: i64,
    last_day: i64,
    start_ms: i64,
    end_ms: i64,
}

impl Window {
    fn new(clock: &Clock, days: i64) -> Window {
        let first_day = clock.today() - 1;
        let last_day = clock.today() + days;
        Window { first_day, last_day, start_ms: clock.start_of(first_day), end_ms: clock.start_of(last_day + 1) }
    }

    fn holds(&self, day: i64) -> bool {
        (self.first_day..=self.last_day).contains(&day)
    }
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn weekday(day_index: i64) -> &'static str {
    ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"][(day_index + 4).rem_euclid(7) as usize]
}

/// The calendar date of a day counted from 1970-01-01, by Howard Hinnant's civil-from-days.
pub fn civil(day_index: i64) -> (i64, u32, u32) {
    let z = day_index + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

/// A moment as the servers' calendars take it, in UTC.
pub fn iso_utc(ms: i64) -> String {
    let (year, month, day) = civil(ms.div_euclid(DAY_MS));
    let secs = ms.rem_euclid(DAY_MS) / 1000;
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", secs / 3600, secs % 3600 / 60, secs % 60)
}

/// "Today" after "Out on digital" reads as "Out on digital today"; a weekday or date keeps its
/// capital.
fn lower_first(when: &str) -> String {
    match when {
        "Today" | "Tomorrow" | "Yesterday" => when.to_lowercase(),
        _ => when.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Tiles
// ---------------------------------------------------------------------------

/// Episodes airing in the window, each with its moment. One already aired and downloaded is
/// left to the New row.
fn upcoming_episodes(episodes: Vec<Episode>, window: &Window, clock: &Clock, open: &dyn Fn(&str) -> Action) -> Vec<(i64, Item)> {
    episodes
        .into_iter()
        .filter_map(|e| {
            let at = e.air_date_utc.as_deref().and_then(parse_iso8601)?;
            let day = clock.day_of(at);
            if !window.holds(day) || (e.has_file && day < clock.today()) {
                return None;
            }
            let series = e.series.clone().unwrap_or_default();
            let when = if e.has_file {
                "Ready to watch".to_string()
            } else if day < clock.today() {
                "Aired yesterday".to_string()
            } else {
                clock.when(day)
            };
            let about = e.occasion().map(str::to_string).or_else(|| e.named_title());
            Some((
                at,
                Item {
                    id: format!("ep:{}", e.id),
                    kind: ItemKind::Program,
                    title: series.title.clone(),
                    subtitle: Some(match about {
                        Some(about) => format!("{when}, {about}"),
                        None => when,
                    }),
                    season: Some(e.season.to_string()),
                    episode: Some(e.episode.to_string()),
                    thumbnail: picture(&e.images, "screenshot").or_else(|| picture(&series.images, "fanart")),
                    poster: picture(&series.images, "poster"),
                    row: Some("upcoming".into()),
                    action: open(&series.title),
                    ..Default::default()
                },
            ))
        })
        .collect()
}

/// The release of a film that falls in the window, with what kind of release it is. A home
/// release in the window counts before a cinema date, since that is what can be watched.
fn release_in(m: &Movie, window: &Window, clock: &Clock) -> Option<(i64, &'static str)> {
    [(&m.digital_release, "Out on digital", 0), (&m.physical_release, "On disc", 0), (&m.in_cinemas, "In cinemas", 1)]
        .into_iter()
        .filter_map(|(date, label, rank)| Some((clock.day_of_date(date.as_deref().and_then(parse_iso8601)?), label, rank)))
        .filter(|(day, _, _)| window.holds(*day))
        .min_by_key(|(day, _, rank)| (*rank, *day))
        .map(|(day, label, _)| (day, label))
}

/// Films with a release in the window, each with the start of its day.
fn upcoming_movies(movies: Vec<Movie>, window: &Window, clock: &Clock, open: &dyn Fn(&str) -> Action) -> Vec<(i64, Item)> {
    movies
        .into_iter()
        .filter_map(|m| {
            let (day, label) = release_in(&m, window, clock)?;
            if m.has_file && day < clock.today() {
                return None;
            }
            Some((
                clock.start_of(day),
                Item {
                    id: format!("film:{}", m.id),
                    kind: ItemKind::Program,
                    title: m.title.clone(),
                    subtitle: Some(if m.has_file { "Ready to watch".to_string() } else { format!("{label} {}", lower_first(&clock.when(day))) }),
                    thumbnail: picture(&m.images, "fanart"),
                    poster: picture(&m.images, "poster"),
                    row: Some("upcoming".into()),
                    action: open(&m.title),
                    ..Default::default()
                },
            ))
        })
        .collect()
}

/// What is in the download queue, one tile per download, so a season pack is one tile.
fn downloading_items(records: Vec<QueueRecord>, open: &dyn Fn(&str) -> Action) -> Vec<Item> {
    let mut groups: Vec<(String, Vec<QueueRecord>)> = Vec::new();
    for r in records {
        let key = r.download_id.clone().filter(|d| !d.is_empty()).or_else(|| r.title.clone()).unwrap_or_default();
        if key.is_empty() {
            continue;
        }
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, list)) => list.push(r),
            None => groups.push((key, vec![r])),
        }
    }
    groups
        .into_iter()
        .filter_map(|(key, list)| {
            let first = list.first()?;
            let state = first.state();
            let (title, poster, fanart) = match (&first.series, &first.movie) {
                (Some(series), _) => (series.title.clone(), picture(&series.images, "poster"), picture(&series.images, "fanart")),
                (None, Some(movie)) => (movie.title.clone(), picture(&movie.images, "poster"), picture(&movie.images, "fanart")),
                (None, None) => (first.title.clone().unwrap_or_default(), None, None),
            };
            if title.is_empty() {
                return None;
            }
            let episodes: Vec<&Episode> = list.iter().filter_map(|r| r.episode.as_ref()).collect();
            let (season, episode, subtitle) = match episodes.as_slice() {
                [one] => (Some(one.season.to_string()), Some(one.episode.to_string()), one.named_title().map(|t| format!("{state}, {t}")).unwrap_or_else(|| state.to_string())),
                [] => (None, None, state.to_string()),
                many => (None, None, format!("{state}, {} episodes", many.len())),
            };
            Some(Item {
                id: format!("dl:{key}"),
                kind: ItemKind::Program,
                title: title.clone(),
                subtitle: Some(subtitle),
                season,
                episode,
                thumbnail: fanart,
                poster,
                row: Some("downloading".into()),
                action: open(&title),
                ..Default::default()
            })
        })
        .collect()
}

/// Episodes that arrived, one tile per show, newest first, so a season pack is one tile.
fn arrived_episodes(records: Vec<EpisodeHistory>, open: &dyn Fn(&str) -> Action) -> Vec<Item> {
    let mut by_series: BTreeMap<i64, (Series, Vec<EpisodeHistory>)> = BTreeMap::new();
    for h in records {
        let Some(series) = h.series.clone() else { continue };
        let id = if h.series_id > 0 { h.series_id } else { series.id };
        by_series.entry(id).or_insert_with(|| (series, Vec::new())).1.push(h);
    }
    by_series
        .into_iter()
        .map(|(id, (series, mut list))| {
            list.sort_by_key(|h| std::cmp::Reverse(h.date.as_deref().and_then(parse_iso8601)));
            let newest = &list[0];
            let episode = newest.episode.clone().unwrap_or_default();
            let (season, number, subtitle) = if list.len() == 1 {
                (Some(episode.season.to_string()), Some(episode.episode.to_string()), episode.named_title())
            } else {
                (None, None, Some(format!("{} new episodes", list.len())))
            };
            Item {
                id: format!("got:series:{id}"),
                kind: ItemKind::Program,
                title: series.title.clone(),
                subtitle,
                season,
                episode: number,
                added_ms: newest.date.as_deref().and_then(parse_iso8601),
                poster: picture(&series.images, "poster"),
                thumbnail: picture(&series.images, "fanart"),
                row: Some("recent".into()),
                action: open(&series.title),
                ..Default::default()
            }
        })
        .collect()
}

/// Films that arrived, one tile each, by their newest import.
fn arrived_movies(records: Vec<MovieHistory>, open: &dyn Fn(&str) -> Action) -> Vec<Item> {
    let mut newest: BTreeMap<i64, (Movie, Option<i64>)> = BTreeMap::new();
    for h in records {
        let Some(m) = h.movie.clone() else { continue };
        let id = if h.movie_id > 0 { h.movie_id } else { m.id };
        let at = h.date.as_deref().and_then(parse_iso8601);
        match newest.get_mut(&id) {
            Some(entry) if entry.1 >= at => {}
            Some(entry) => *entry = (m, at),
            None => {
                newest.insert(id, (m, at));
            }
        }
    }
    newest
        .into_iter()
        .map(|(id, (m, at))| Item {
            id: format!("got:film:{id}"),
            kind: ItemKind::Program,
            title: m.title.clone(),
            subtitle: m.year.map(|y| y.to_string()),
            added_ms: at,
            poster: picture(&m.images, "poster"),
            thumbnail: picture(&m.images, "fanart"),
            row: Some("recent".into()),
            action: open(&m.title),
            ..Default::default()
        })
        .collect()
}

/// The TV's own search for a title, as an intent Lounge lets through.
pub fn tv_search_action(title: &str) -> Action {
    Action { view: Some(format!("intent:#Intent;action=android.search.action.GLOBAL_SEARCH;S.query={};end", uri_encode(title))), ..Default::default() }
}

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

    fn at(s: &str) -> i64 {
        parse_iso8601(s).unwrap()
    }

    /// A Monday evening in Sydney, which is still Monday morning in UTC.
    fn sydney() -> Clock {
        Clock { now_ms: at("2026-11-09T09:00:00Z"), utc_offset_minutes: 600 }
    }

    #[test]
    fn days_are_named_from_the_tvs_own_day() {
        let clock = sydney();
        assert_eq!(clock.when(clock.day_of(at("2026-11-09T12:00:00Z"))), "Today");
        assert_eq!(clock.when(clock.day_of(at("2026-11-09T14:30:00Z"))), "Tomorrow");
        assert_eq!(clock.when(clock.day_of(at("2026-11-08T12:00:00Z"))), "Yesterday");
        assert_eq!(clock.when(clock.day_of(at("2026-11-12T12:00:00Z"))), "Thursday");
        assert_eq!(clock.when(clock.day_of(at("2026-11-15T12:00:00Z"))), "Sunday");
        assert_eq!(clock.when(clock.day_of(at("2026-11-16T12:00:00Z"))), "16 Nov");
        assert_eq!(clock.when(clock.day_of(at("2026-11-01T12:00:00Z"))), "1 Nov");
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(at("2000-03-01T00:00:00Z") / DAY_MS), (2000, 3, 1));
        assert_eq!(iso_utc(at("2026-11-09T09:05:07Z")), "2026-11-09T09:05:07Z");
        assert_eq!(weekday(0), "Thursday");
        assert_eq!(lower_first("Today"), "today");
        assert_eq!(lower_first("Friday"), "Friday");
    }

    #[test]
    fn the_window_is_whole_days_of_the_tvs_time() {
        let clock = sydney();
        let w = Window::new(&clock, 7);
        // Local midnight on Sunday 8 Nov in Sydney is 14:00 UTC on the 7th.
        assert_eq!(iso_utc(w.start_ms), "2026-11-07T14:00:00Z");
        assert_eq!(iso_utc(w.end_ms), "2026-11-16T14:00:00Z");
        assert!(w.holds(clock.today() - 1) && w.holds(clock.today() + 7) && !w.holds(clock.today() + 8) && !w.holds(clock.today() - 2));
        // A date Radarr stores as midnight UTC, or as midnight in its own zone, is that date.
        assert_eq!(clock.day_of_date(at("2026-11-12T00:00:00Z")), clock.day_of_date(at("2026-11-11T19:00:00Z")));
        assert_eq!(civil(clock.day_of_date(at("2026-11-12T00:00:00Z"))), (2026, 11, 12));
    }

    #[test]
    fn calendar_episodes_become_tiles_with_the_day_first() {
        let episodes: Vec<Episode> = serde_json::from_str(
            r#"[{"id":12,"title":"Honeydew","seasonNumber":2,"episodeNumber":4,"airDateUtc":"2026-11-11T01:00:00Z","hasFile":false,"monitored":true,
                 "series":{"id":1,"title":"The Bear","year":2022,"images":[{"coverType":"poster","url":"/MediaCover/1/poster.jpg","remoteUrl":"https://artworks.thetvdb.com/p.jpg"},{"coverType":"fanart","remoteUrl":"https://artworks.thetvdb.com/f.jpg"}]},
                 "images":[{"coverType":"screenshot","remoteUrl":"https://artworks.thetvdb.com/e.jpg"}]},
                {"id":7,"title":"TBA","seasonNumber":1,"episodeNumber":1,"airDateUtc":"2026-11-08T20:00:00Z","hasFile":true,"series":{"title":"Older","images":[]}},
                {"id":8,"title":"Pilot","seasonNumber":1,"episodeNumber":1,"airDateUtc":"2026-11-08T20:00:00Z","hasFile":false,"series":{"title":"Missed","images":[]}},
                {"id":10,"title":"Last","seasonNumber":3,"episodeNumber":9,"airDateUtc":"2026-11-10T01:00:00Z","finaleType":"season","series":{"title":"Ending"}},
                {"id":11,"title":"TBA","seasonNumber":4,"episodeNumber":1,"airDateUtc":"2026-11-10T02:00:00Z","series":{"title":"Returning"}},
                {"id":9,"title":"No date","seasonNumber":1,"episodeNumber":2,"series":{"title":"Dateless"}},
                {"id":13,"title":"Far","seasonNumber":1,"episodeNumber":3,"airDateUtc":"2026-12-01T01:00:00Z","series":{"title":"Later"}}]"#,
        )
        .unwrap();
        let clock = Clock { now_ms: at("2026-11-09T09:00:00Z"), utc_offset_minutes: 0 };
        let window = Window::new(&clock, 7);
        let mut items = upcoming_episodes(episodes, &window, &clock, &|t| Action::search(t));
        items.sort_by_key(|(at, _)| *at);
        let got: Vec<(&str, Option<&str>)> = items.iter().map(|(_, it)| (it.id.as_str(), it.subtitle.as_deref())).collect();
        assert_eq!(got, [("ep:8", Some("Aired yesterday, Series premiere")), ("ep:10", Some("Tomorrow, Season finale")), ("ep:11", Some("Tomorrow, Season premiere")), ("ep:12", Some("Wednesday, Honeydew"))]);
        let bear = &items[3].1;
        assert_eq!(bear.title, "The Bear");
        assert_eq!((bear.season.as_deref(), bear.episode.as_deref()), (Some("2"), Some("4")));
        assert_eq!(bear.thumbnail.as_deref(), Some("https://artworks.thetvdb.com/e.jpg"));
        assert_eq!(bear.poster.as_deref(), Some("https://artworks.thetvdb.com/p.jpg"));
        assert_eq!(bear.row.as_deref(), Some("upcoming"));
        assert_eq!(bear.action.search.as_deref(), Some("The Bear"));
    }

    #[test]
    fn a_films_home_release_in_the_window_is_the_one_named_on_the_right_day() {
        let movies: Vec<Movie> = serde_json::from_str(
            r#"[{"id":1,"title":"Soon","year":2026,"inCinemas":"2026-10-01T00:00:00Z","digitalRelease":"2026-11-12T00:00:00Z","physicalRelease":"2026-12-20T00:00:00Z","hasFile":false,
                 "images":[{"coverType":"poster","remoteUrl":"https://image.tmdb.org/t/p/original/p.jpg"},{"coverType":"fanart","remoteUrl":"https://image.tmdb.org/t/p/original/f.jpg"}]},
                {"id":2,"title":"Cinema","inCinemas":"2026-11-09T00:00:00Z","hasFile":false},
                {"id":3,"title":"Got","digitalRelease":"2026-11-10T00:00:00Z","hasFile":true},
                {"id":5,"title":"Both","inCinemas":"2026-11-08T00:00:00Z","digitalRelease":"2026-11-13T00:00:00Z"},
                {"id":6,"title":"Seen","digitalRelease":"2026-11-08T00:00:00Z","hasFile":true},
                {"id":4,"title":"Far off","digitalRelease":"2027-01-01T00:00:00Z"}]"#,
        )
        .unwrap();
        // Monday evening in New York: Radarr's midnight-UTC dates must still read as their date.
        let clock = Clock { now_ms: at("2026-11-10T01:00:00Z"), utc_offset_minutes: -300 };
        let window = Window::new(&clock, 7);
        let mut items = upcoming_movies(movies, &window, &clock, &|t| tv_search_action(t));
        items.sort_by_key(|(at, _)| *at);
        let got: Vec<(&str, Option<&str>)> = items.iter().map(|(_, it)| (it.title.as_str(), it.subtitle.as_deref())).collect();
        assert_eq!(got, [("Cinema", Some("In cinemas today")), ("Got", Some("Ready to watch")), ("Soon", Some("Out on digital Thursday")), ("Both", Some("Out on digital Friday"))]);
        let soon = &items[2].1;
        assert_eq!(soon.poster.as_deref(), Some("https://image.tmdb.org/t/p/w500/p.jpg"));
        assert_eq!(soon.thumbnail.as_deref(), Some("https://image.tmdb.org/t/p/w1280/f.jpg"));
        assert_eq!(items[0].1.action.view.as_deref(), Some("intent:#Intent;action=android.search.action.GLOBAL_SEARCH;S.query=Cinema;end"));
    }

    #[test]
    fn a_season_pack_arrives_as_one_tile() {
        let page: HistoryPage<EpisodeHistory> = serde_json::from_str(
            r#"{"page":1,"records":[
                {"episodeId":12,"seriesId":1,"date":"2026-11-08T22:00:00Z","eventType":"downloadFolderImported","episode":{"id":12,"title":"Honeydew","seasonNumber":2,"episodeNumber":4},"series":{"id":1,"title":"The Bear","images":[{"coverType":"poster","remoteUrl":"https://a/p.jpg"}]}},
                {"episodeId":31,"seriesId":3,"date":"2026-11-08T23:00:00Z","episode":{"id":31,"title":"One","seasonNumber":1,"episodeNumber":1},"series":{"id":3,"title":"Pack"}},
                {"episodeId":32,"seriesId":3,"date":"2026-11-08T23:00:01Z","episode":{"id":32,"title":"Two","seasonNumber":1,"episodeNumber":2},"series":{"id":3,"title":"Pack"}},
                {"episodeId":33,"seriesId":3,"date":"2026-11-08T23:00:02Z","episode":{"id":33,"title":"Three","seasonNumber":1,"episodeNumber":3},"series":{"id":3,"title":"Pack"}},
                {"episodeId":13,"date":"2026-11-08T22:00:00Z"}]}"#,
        )
        .unwrap();
        let mut items = arrived_episodes(page.records, &|t| Action::search(t));
        items.sort_by_key(|it| it.id.clone());
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "got:series:1");
        assert_eq!(items[0].subtitle.as_deref(), Some("Honeydew"));
        assert_eq!((items[0].season.as_deref(), items[0].episode.as_deref()), (Some("2"), Some("4")));
        assert_eq!(items[0].added_ms, parse_iso8601("2026-11-08T22:00:00Z"));
        assert_eq!(items[0].row.as_deref(), Some("recent"));
        assert_eq!(items[1].id, "got:series:3");
        assert_eq!(items[1].subtitle.as_deref(), Some("3 new episodes"));
        assert_eq!((items[1].season.as_deref(), items[1].episode.as_deref()), (None, None));
        assert_eq!(items[1].added_ms, parse_iso8601("2026-11-08T23:00:02Z"));
        let films: HistoryPage<MovieHistory> = serde_json::from_str(
            r#"{"records":[{"movieId":5,"date":"2026-11-07T10:00:00Z","movie":{"id":5,"title":"Static Bloom","year":2021,"images":[]}},
                            {"movieId":5,"date":"2026-11-06T10:00:00Z","movie":{"id":5,"title":"Static Bloom","year":2021,"images":[]}}]}"#,
        )
        .unwrap();
        let films = arrived_movies(films.records, &|t| Action::search(t));
        assert_eq!(films.len(), 1);
        assert_eq!((films[0].id.as_str(), films[0].subtitle.as_deref(), films[0].added_ms), ("got:film:5", Some("2021"), parse_iso8601("2026-11-07T10:00:00Z")));
        assert!(<Arr as sdk::Plugin>::manifest().rows.iter().any(|r| r.id == "recent"));
    }

    #[test]
    fn the_queue_is_one_tile_per_download() {
        let page: QueuePage<QueueRecord> = serde_json::from_str(
            r#"{"records":[
                {"downloadId":"ABC","title":"Pack.S01","status":"downloading","trackedDownloadState":"downloading","series":{"id":3,"title":"Pack","images":[{"coverType":"poster","remoteUrl":"https://a/p.jpg"}]},"episode":{"id":31,"title":"One","seasonNumber":1,"episodeNumber":1}},
                {"downloadId":"ABC","title":"Pack.S01","status":"downloading","trackedDownloadState":"downloading","series":{"id":3,"title":"Pack"},"episode":{"id":32,"title":"Two","seasonNumber":1,"episodeNumber":2}},
                {"downloadId":"DEF","status":"completed","trackedDownloadState":"importPending","series":{"id":1,"title":"The Bear"},"episode":{"id":12,"title":"Honeydew","seasonNumber":2,"episodeNumber":4}},
                {"downloadId":"GHI","status":"paused","movie":{"id":5,"title":"Static Bloom","images":[]}},
                {"downloadId":"JKL","status":"queued","title":"Nameless.Release"},
                {"status":"downloading"}]}"#,
        )
        .unwrap();
        let items = downloading_items(page.records, &|t| Action::search(t));
        let got: Vec<(&str, &str, Option<&str>)> = items.iter().map(|it| (it.id.as_str(), it.title.as_str(), it.subtitle.as_deref())).collect();
        assert_eq!(got, [("dl:ABC", "Pack", Some("Downloading, 2 episodes")), ("dl:DEF", "The Bear", Some("Arriving, Honeydew")), ("dl:GHI", "Static Bloom", Some("Paused")), ("dl:JKL", "Nameless.Release", Some("Waiting"))]);
        assert_eq!(items[0].poster.as_deref(), Some("https://a/p.jpg"));
        assert_eq!((items[1].season.as_deref(), items[1].episode.as_deref()), (Some("2"), Some("4")));
        assert_eq!(items[0].row.as_deref(), Some("downloading"));
    }

    #[test]
    fn pictures_come_from_public_addresses_only() {
        let images: Vec<Image> = serde_json::from_str(r#"[{"coverType":"poster","url":"/MediaCover/1/poster.jpg"},{"coverType":"fanart","remoteUrl":" https://a/f.jpg "},{"coverType":"banner","remoteUrl":"javascript:x"}]"#).unwrap();
        assert_eq!(picture(&images, "poster"), None);
        assert_eq!(picture(&images, "fanart").as_deref(), Some("https://a/f.jpg"));
        assert_eq!(picture(&images, "banner"), None);
        assert_eq!(uri_encode("The Bear & co"), "The%20Bear%20%26%20co");
    }

    #[test]
    fn errors_name_the_server_and_what_to_check() {
        let s = Server { name: "Sonarr", url: "http://192.168.0.20:8989".into(), key: "k".into() };
        // Off the TV there is no host, so every fetch fails as an unreachable server would.
        assert_eq!(s.get_json::<Vec<Episode>>("/api/v3/calendar").err().as_deref(), Some("Sonarr didn't answer at 192.168.0.20:8989. Check the server, then try again."));
        assert_eq!(s.check_identity().err().as_deref(), Some("Sonarr didn't answer at 192.168.0.20:8989. Check the server, then try again."));
        assert!(Server::from_settings("Radarr", "radarr", "radarr_key").unwrap().is_none());
        assert!(<Arr as sdk::Plugin>::manifest().settings.iter().all(|s| !s.required));
        let status: Status = serde_json::from_str(r#"{"appName":"Radarr","version":"5.0"}"#).unwrap();
        assert_eq!(status.app_name, "Radarr");
    }
}
