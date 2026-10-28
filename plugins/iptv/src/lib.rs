//! IPTV plugin: an M3U playlist becomes channels in Live now and the guide; an optional XMLTV
//! feed (plain or gzipped) fills in what is on. Streams open in whatever video player the TV
//! has (`play` action).

use lounge_plugin_sdk::{self as sdk, epoch_ms, host, Action, HttpRequest, Item, ItemKind, Kind, Manifest, RefreshContext, SettingKind, SettingSpec};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::hash::BuildHasherDefault;
use std::io::Read;

pub struct Iptv;

/// Most bytes a gzipped guide may unpack to; reading stops there.
const MAX_GUIDE_BYTES: usize = 48 * 1024 * 1024;
/// How long reading the guide may take before what has been read so far is used.
const GUIDE_MS: i64 = 20_000;
/// Longest text read from one guide field, in bytes.
const MAX_FIELD_BYTES: usize = 1024;
/// Longest description shown under a programme, in characters.
const DESCRIPTION_CHARS: usize = 140;

impl sdk::Plugin for Iptv {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.iptv".into(),
            name: "IPTV playlist".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Channels from an M3U playlist, with programme information from an XMLTV guide.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Epg],
            fetch_hosts: vec!["$playlist_url".into(), "$epg_url".into(), "*".into()],
            refresh_minutes: 60,
            rows: Vec::new(),
            settings: vec![
                SettingSpec { key: "playlist_url".into(), label: "Playlist URL (M3U)".into(), kind: SettingKind::SecretUrl, required: true, help: Some("The .m3u or .m3u8 playlist your provider gave you.".into()), default: None },
                SettingSpec { key: "epg_url".into(), label: "Guide URL (XMLTV)".into(), kind: SettingKind::SecretUrl, required: false, help: Some("Optional. Plain or .gz XMLTV; channel ids must match the playlist's tvg-id.".into()), default: None },
                SettingSpec { key: "group".into(), label: "Only this group".into(), kind: SettingKind::Text, required: false, help: Some("Optional group-title filter, for very large playlists.".into()), default: None },
            ],
            homepage: None,
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        // Provider links often carry the account's user name and password, so both are secrets.
        let playlist_url = host::secret("playlist_url").filter(|v| !v.trim().is_empty()).ok_or("Playlist URL is not set")?;
        let group = host::setting("group");
        let (channels, total) = parse_m3u(&host::get_text(&playlist_url)?, group.as_deref(), sdk::MAX_ITEMS);
        if channels.is_empty() {
            return Err(match group {
                Some(g) => format!("the playlist has no channels in the group \"{g}\""),
                None => "the playlist has no channels".into(),
            });
        }
        if total > channels.len() {
            sdk::warn!("the playlist has {total} channels and the TV shows the first {}; choose a group in the settings to see others", channels.len());
        }
        sdk::info!("{} channels in playlist", channels.len());

        let mut items: Vec<Item> = channels.iter().map(Channel::to_item).collect();
        let budget = sdk::MAX_ITEMS - items.len();
        match host::secret("epg_url") {
            Some(_) if budget == 0 => sdk::warn!("the channels fill all {} places the TV keeps, so the guide is left out", sdk::MAX_ITEMS),
            Some(epg_url) => match guide(&epg_url, &channels, ctx, budget) {
                Ok(schedule) => items.extend(schedule),
                Err(e) if sdk::last_guide_still_showing(ctx) => {
                    sdk::warn!("guide unavailable: {e}");
                    return Err(sdk::GUIDE_KEPT.into());
                }
                Err(e) => sdk::warn!("guide unavailable, showing the channels without it: {e}"),
            },
            None => {}
        }
        Ok(items)
    }
}

sdk::export_plugin!(Iptv);

/// Fetches the guide and reads it as it unpacks, keeping only what fits in `budget` items.
fn guide(url: &str, channels: &[Channel], ctx: &RefreshContext, budget: usize) -> Result<Vec<Item>, String> {
    let response = host::fetch(&HttpRequest::get(url))?;
    if !response.ok() {
        return Err(format!("HTTP {} for the guide", response.status));
    }
    let bytes = sdk::body_bytes(response);
    let window = (ctx.now_ms, ctx.now_ms + i64::from(ctx.window_hours.max(1)) * 3_600_000);
    let mut guide = Guide::new(channels, window, budget);
    guide.deadline_ms = host::now_ms() + GUIDE_MS;
    let outcome = read_guide(&bytes, &mut guide, MAX_GUIDE_BYTES);
    drop(bytes);
    if let Err(e) = outcome {
        if guide.stored == 0 {
            return Err(e);
        }
        sdk::warn!("the guide stopped early ({e}); keeping what came before");
    }
    if guide.seen > 0 && guide.matched == 0 {
        sdk::warn!("no guide channel ids match the playlist's tvg-id values or names");
    }
    let (seen, aliases) = (guide.seen, guide.aliases.len());
    let schedule = guide.into_items();
    sdk::info!("{} of {seen} guide programmes fit, for {} channels ({aliases} guide channels matched by name)", schedule.len(), channels.len());
    Ok(schedule)
}

/// Reads a plain or gzipped XMLTV document into `guide`, unpacking at most `cap` bytes.
pub fn read_guide(bytes: &[u8], guide: &mut Guide, cap: usize) -> Result<(), String> {
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return parse_xmltv(bytes, guide);
    }
    let mut input = Capped { inner: flate2::read::MultiGzDecoder::new(bytes), left: cap, over: false };
    parse_xmltv(&mut input, guide).map_err(|e| if input.over { format!("it unpacks to more than {} MB", cap >> 20) } else { e })
}

/// Passes on at most `left` bytes and then fails, so a guide that unpacks without end stops.
struct Capped<R> {
    inner: R,
    left: usize,
    over: bool,
}

impl<R: Read> Read for Capped<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.left == 0 {
            if self.inner.read(&mut [0u8])? == 0 {
                return Ok(0);
            }
            self.over = true;
            return Err(std::io::Error::other("the guide is too large"));
        }
        let n = buf.len().min(self.left);
        let got = self.inner.read(&mut buf[..n])?;
        self.left -= got;
        Ok(got)
    }
}

/// One playlist entry. `id` is unique in the playlist and stays the same between refreshes:
/// the tvg-id, else the name as a slug, else a hash of the stream address, with "~2", "~3" and
/// so on added when one repeats.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub id: String,
    /// The tvg-id as written, which guide programmes refer to; repeats share it.
    pub tvg_id: Option<String>,
    pub name: String,
    /// The name the guide knows the channel by, when the playlist gives one besides its title.
    pub tvg_name: Option<String>,
    pub number: Option<String>,
    pub logo: Option<String>,
    pub group: Option<String>,
    pub url: String,
}

impl Channel {
    fn to_item(&self) -> Item {
        Item {
            id: format!("ch:{}", self.id),
            kind: ItemKind::Channel,
            title: self.name.clone(),
            subtitle: self.group.clone(),
            logo: self.logo.clone(),
            channel_id: Some(self.id.clone()),
            channel_name: Some(self.name.clone()),
            channel_number: self.number.clone(),
            action: Action::play(&self.url, &self.name),
            ..Default::default()
        }
    }

    fn guide_id(&self) -> &str {
        self.tvg_id.as_deref().unwrap_or(&self.id)
    }
}

/// Films and series an Xtream playlist lists beside its channels, which play as files, not live.
fn is_on_demand(url: &str) -> bool {
    let path = url.split(['?', '#', '|']).next().unwrap_or(url).to_ascii_lowercase();
    path.contains("/movie/") || path.contains("/series/")
}

/// Parse an extended M3U, keeping the first `limit` channels in `only_group` (every group when
/// `None`); also returns how many channels the group has. Attributes on `#EXTINF` (`tvg-id`,
/// `tvg-name`, `tvg-logo`, `tvg-chno`, `group-title`) are optional.
pub fn parse_m3u(text: &str, only_group: Option<&str>, limit: usize) -> (Vec<Channel>, usize) {
    let mut out = Vec::new();
    let mut used = HashSet::new();
    let mut total = 0;
    let mut pending: Option<(HashMap<String, String>, String)> = None;
    let mut index = 0usize;
    for raw in text.lines() {
        // A byte order mark can open the file even where #EXTM3U is missing.
        let line = raw.trim_start_matches('\u{feff}').trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let (attrs, name) = split_extinf(rest);
            pending = Some((parse_attrs(attrs), name.to_string()));
        } else if line.starts_with('#') {
            continue;
        } else if let Some((attrs, name)) = pending.take() {
            if is_on_demand(line) {
                continue;
            }
            index += 1;
            let name = if name.is_empty() { attrs.get("tvg-name").cloned().filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Channel {index}")) } else { name };
            let tvg_id = attrs.get("tvg-id").map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            let base = tvg_id.clone().unwrap_or_else(|| Some(slug(&name)).filter(|s| !s.is_empty()).unwrap_or_else(|| url_hash(line)));
            let id = unique_id(&mut used, base);
            let group = attrs.get("group-title").cloned().filter(|g| !g.is_empty());
            if only_group.is_some_and(|want| !group.as_deref().is_some_and(|g| g.eq_ignore_ascii_case(want))) {
                continue;
            }
            total += 1;
            if out.len() < limit {
                out.push(Channel {
                    id,
                    tvg_id,
                    tvg_name: attrs.get("tvg-name").map(|n| n.trim().to_string()).filter(|n| !n.is_empty() && *n != name),
                    name,
                    number: attrs.get("tvg-chno").cloned().filter(|s| !s.is_empty()).or_else(|| Some(index.to_string())),
                    logo: attrs.get("tvg-logo").cloned().filter(|s| s.starts_with("http")),
                    group,
                    url: line.to_string(),
                });
            }
        }
    }
    (out, total)
}

/// Splits what follows `#EXTINF:` into the duration with its attributes, and the title, which
/// is everything after the first comma outside quotes.
fn split_extinf(rest: &str) -> (&str, &str) {
    let mut quote: Option<char> = None;
    for (i, c) in rest.char_indices() {
        match (c, quote) {
            ('"' | '\'', None) => quote = Some(c),
            (c, Some(q)) if c == q => quote = None,
            (',', None) => return (&rest[..i], rest[i + 1..].trim()),
            _ => {}
        }
    }
    match rest.rfind(',') {
        Some(i) => (&rest[..i], rest[i + 1..].trim()),
        None => (rest, ""),
    }
}

fn parse_attrs(s: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let key_start = i;
        while i < bytes.len() && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let key = &s[key_start..i];
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        let value = if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
            let quote = bytes[i];
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            let v = &s[start..i.min(bytes.len())];
            i += 1;
            v
        } else {
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            &s[start..i]
        };
        if !key.is_empty() {
            out.insert(key.to_ascii_lowercase(), value.to_string());
        }
    }
    out
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// A hash of the stream address that stays the same across builds and platforms.
fn url_hash(url: &str) -> String {
    let mut hash = Fnv::default();
    std::hash::Hasher::write(&mut hash, url.as_bytes());
    format!("url-{:016x}", std::hash::Hasher::finish(&hash))
}

fn unique_id(used: &mut HashSet<String>, base: String) -> String {
    let mut id = base.clone();
    let mut n = 1;
    while !used.insert(id.clone()) {
        n += 1;
        id = format!("{base}~{n}");
    }
    id
}

pub fn normalise(name: &str) -> String {
    let lower = name.to_lowercase();
    let cut = lower.split(['(', '[']).next().unwrap_or("");
    cut.chars().filter(|c| c.is_alphanumeric()).collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Programme {
    pub title: String,
    pub description: Option<String>,
    pub start_ms: i64,
    pub end_ms: i64,
    /// The guide gave no stop, so it ends where the next programme on its channel starts.
    open: bool,
}

/// The end of a programme with no stop and no programme after it yet.
const OPEN_END: i64 = i64::MAX;

impl Programme {
    fn to_item(&self, ch: &Channel) -> Item {
        Item {
            id: format!("prog:{}:{}", ch.id, self.start_ms),
            kind: ItemKind::Schedule,
            title: self.title.clone(),
            subtitle: self.description.clone(),
            duration_ms: Some(self.end_ms - self.start_ms),
            channel_id: Some(ch.id.clone()),
            channel_name: Some(ch.name.clone()),
            channel_number: ch.number.clone(),
            start_ms: Some(self.start_ms),
            end_ms: Some(self.end_ms),
            ..Default::default()
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max - 1).collect::<String>().trim_end()) }
}

/// Programmes gathered from a guide for the playlist's channels. Programmes are kept per
/// bucket, the playlist channels that share one guide id, and trimmed as they arrive so no
/// more are held than could be shown.
pub struct Guide<'a> {
    channels: &'a [Channel],
    by_id: FastMap<&'a str, usize>,
    by_name: FastMap<String, usize>,
    /// Guide channel ids in lower case, for guides that write them differently.
    by_folded_id: FastMap<String, usize>,
    /// Guide channel ids that match no tvg-id exactly, matched to a bucket by its id in another
    /// case or by display name.
    aliases: FastMap<String, usize>,
    /// Indices into `channels` for each bucket; several when a tvg-id repeats.
    buckets: Vec<Vec<usize>>,
    programmes: Vec<Vec<Programme>>,
    stored: usize,
    /// How many programmes a bucket can still use, known after the first trim.
    keep: usize,
    /// When reading stops, in epoch ms.
    pub deadline_ms: i64,
    window: (i64, i64),
    budget: usize,
    /// Programmes read, and how many of them belong to a playlist channel.
    pub seen: usize,
    pub matched: usize,
}

/// A programme whose element is still being read.
struct Pending {
    bucket: usize,
    start_ms: i64,
    end_ms: i64,
    title: Option<String>,
    sub_title: Option<String>,
    desc: Option<String>,
}

impl<'a> Guide<'a> {
    /// Programmes overlapping `window` (start and end in epoch ms) for `channels`, to fill at
    /// most `budget` items.
    pub fn new(channels: &'a [Channel], window: (i64, i64), budget: usize) -> Guide<'a> {
        let mut by_id = FastMap::default();
        let mut by_folded_id = FastMap::default();
        let mut by_name = FastMap::default();
        let mut buckets: Vec<Vec<usize>> = Vec::new();
        for (i, c) in channels.iter().enumerate() {
            let bucket = *by_id.entry(c.guide_id()).or_insert_with(|| {
                buckets.push(Vec::new());
                buckets.len() - 1
            });
            buckets[bucket].push(i);
            by_folded_id.entry(c.guide_id().to_lowercase()).or_insert(bucket);
            for name in std::iter::once(&c.name).chain(&c.tvg_name).map(|n| normalise(n)) {
                if !name.is_empty() {
                    by_name.entry(name).or_insert(bucket);
                }
            }
        }
        let programmes = vec![Vec::new(); buckets.len()];
        Guide { channels, by_id, by_folded_id, by_name, aliases: FastMap::default(), buckets, programmes, stored: 0, keep: usize::MAX, deadline_ms: i64::MAX, window, budget, seen: 0, matched: 0 }
    }

    fn alias(&mut self, guide_id: &str, display_name: &str) {
        if self.by_id.contains_key(guide_id) {
            return;
        }
        if let Some(&bucket) = self.by_folded_id.get(&guide_id.to_lowercase()).or_else(|| self.by_name.get(&normalise(display_name))) {
            self.aliases.entry(guide_id.to_string()).or_insert(bucket);
        }
    }

    /// The programme a `<programme …>` start tag opens, when it belongs to a playlist channel
    /// and overlaps the window.
    fn start(&mut self, tag: &[u8]) -> Option<Pending> {
        self.seen += 1;
        let (mut channel, mut start, mut stop) = (None, None, None);
        for (name, value) in Attributes(tag) {
            match name {
                b"channel" if value.contains(&b'&') => channel = Some(Cow::Owned(decode(value, MAX_FIELD_BYTES))),
                b"channel" => channel = Some(String::from_utf8_lossy(value)),
                b"start" => start = std::str::from_utf8(value).ok().and_then(parse_xmltv_time),
                b"stop" => stop = std::str::from_utf8(value).ok().and_then(parse_xmltv_time),
                _ => {}
            }
        }
        let channel = channel?;
        let bucket = *self.by_id.get(&*channel).or_else(|| self.aliases.get(&*channel))?;
        self.matched += 1;
        // XMLTV's stop is optional; such a programme is closed off once the next one is known.
        let (start_ms, end_ms) = (start?, stop.unwrap_or(OPEN_END));
        if end_ms <= self.window.0 || start_ms >= self.window.1 || end_ms <= start_ms {
            return None;
        }
        // Once a bucket holds as many earlier programmes as it can use, later ones are not read.
        let list = &self.programmes[bucket];
        if list.len() >= self.keep && list.iter().filter(|p| p.start_ms <= start_ms).count() >= self.keep {
            return None;
        }
        Some(Pending { bucket, start_ms, end_ms, title: None, sub_title: None, desc: None })
    }

    fn add(&mut self, p: Pending) {
        let Some(title) = p.title else { return };
        if self.keep != usize::MAX && self.programmes[p.bucket].iter().any(|q| q.start_ms == p.start_ms) {
            return;
        }
        let description = p.desc.or(p.sub_title).map(|d| truncate(&d, DESCRIPTION_CHARS));
        self.programmes[p.bucket].push(Programme { title, description, start_ms: p.start_ms, end_ms: p.end_ms, open: p.end_ms == OPEN_END });
        self.stored += 1;
        if self.stored > 2 * self.budget + self.buckets.len() + 256 {
            self.trim();
        }
    }

    /// Sorts each bucket and drops what can no longer be shown. The fair share per channel only
    /// shrinks as programmes arrive, and a channel never ends up with more than one past it, so
    /// anything beyond that is safe to drop now.
    fn trim(&mut self) {
        let from = self.window.0;
        for list in &mut self.programmes {
            list.sort_by_key(|p| p.start_ms);
            list.dedup_by_key(|p| p.start_ms);
            for i in 0..list.len() {
                if list[i].open {
                    list[i].end_ms = list.get(i + 1).map_or(OPEN_END, |next| next.start_ms);
                }
            }
            list.retain(|p| p.end_ms > from);
        }
        let counts: Vec<usize> = self.programmes.iter().zip(&self.buckets).flat_map(|(list, twins)| std::iter::repeat_n(list.len(), twins.len())).collect();
        self.keep = sdk::fair_share(&counts, self.budget) + 1;
        for list in &mut self.programmes {
            list.truncate(self.keep);
        }
        self.stored = self.programmes.iter().map(Vec::len).sum();
    }

    /// Schedule items for every playlist channel, at most `budget` of them, shared evenly.
    pub fn into_items(mut self) -> Vec<Item> {
        self.trim();
        // A last programme with no stop has no known end, so it is left out.
        for list in &mut self.programmes {
            list.retain(|p| p.end_ms != OPEN_END);
        }
        let mut items = Vec::new();
        for (list, twins) in self.programmes.iter().zip(&self.buckets) {
            for &t in twins {
                items.extend(list.iter().map(|p| p.to_item(&self.channels[t])));
            }
        }
        sdk::fit_schedule(items, self.budget)
    }
}

/// Reads an XMLTV document into `guide` as it streams in. Elements are found with a plain byte
/// scan, and only the parts of channels and wanted programmes that matter are decoded, which
/// keeps a large guide affordable. On a fault, what came before it is kept and the fault
/// returned.
pub fn parse_xmltv<R: Read>(input: R, guide: &mut Guide) -> Result<(), String> {
    let mut s = Stream { input, buf: Vec::with_capacity(2 * CHUNK), pos: 0, done: false };
    while let Some(lt) = s.find(0, b"<")? {
        s.pos += lt;
        while s.buf.len() - s.pos < 16 && s.more()? {}
        let head = &s.buf[s.pos..];
        if head.starts_with(b"<!--") {
            let end = s.find(4, b"-->")?.ok_or("the guide ends inside a comment")?;
            s.pos += end + 3;
        } else if head.starts_with(b"<![CDATA[") {
            let end = s.find(9, b"]]>")?.ok_or("the guide ends inside a CDATA section")?;
            s.pos += end + 3;
        } else if is_tag(head, b"programme") || is_tag(head, b"channel") {
            let is_programme = is_tag(head, b"programme");
            if guide.seen % 1024 == 1023 && host::now_ms() > guide.deadline_ms {
                return Err(format!("reading it took more than {} seconds", GUIDE_MS / 1000));
            }
            let tag_end = s.tag_end()?;
            let pending = if is_programme { guide.start(&s.buf[s.pos..s.pos + tag_end]) } else { None };
            let len = if s.buf[s.pos + tag_end - 1] == b'/' {
                tag_end + 1
            } else {
                let close: &[u8] = if is_programme { b"</programme" } else { b"</channel" };
                let at = s.find(tag_end, close)?.ok_or("the guide ends part-way through an element")?;
                s.find(at, b">")?.ok_or("the guide ends part-way through an element")? + 1
            };
            let element = &s.buf[s.pos..s.pos + len];
            match pending {
                Some(mut p) => {
                    for (name, content) in Children(element, tag_end + 1) {
                        let slot = match name {
                            b"title" => &mut p.title,
                            b"sub-title" => &mut p.sub_title,
                            b"desc" => &mut p.desc,
                            _ => continue,
                        };
                        if slot.is_none() {
                            *slot = Some(clean(&decode(content, MAX_FIELD_BYTES))).filter(|v| !v.is_empty());
                        }
                    }
                    guide.add(p);
                }
                None if !is_programme => {
                    let id = Attributes(&element[..tag_end]).find(|(name, _)| *name == b"id").map(|(_, v)| decode(v, MAX_FIELD_BYTES));
                    if let Some(id) = id {
                        for (_, content) in Children(element, tag_end + 1).filter(|(name, _)| *name == b"display-name") {
                            guide.alias(&id, &clean(&decode(content, MAX_FIELD_BYTES)));
                        }
                    }
                }
                None => {}
            }
            s.pos += len;
        } else {
            s.pos += s.tag_end()? + 1;
        }
    }
    Ok(())
}

const CHUNK: usize = 64 * 1024;
/// Largest single element the scanner holds while looking for its end.
const MAX_ELEMENT: usize = 1 << 20;

/// A window on the document as it arrives. Offsets handed out are relative to `pos`, which
/// stay valid when consumed bytes are dropped from the front.
struct Stream<R> {
    input: R,
    buf: Vec<u8>,
    pos: usize,
    done: bool,
}

impl<R: Read> Stream<R> {
    /// Reads the next chunk; false at the end of the document.
    fn more(&mut self) -> Result<bool, String> {
        if self.done {
            return Ok(false);
        }
        if self.buf.len() - self.pos > MAX_ELEMENT {
            return Err("an element in the guide is larger than 1 MB".into());
        }
        if self.pos > CHUNK {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
        let len = self.buf.len();
        self.buf.resize(len + CHUNK, 0);
        let read = loop {
            match self.input.read(&mut self.buf[len..]) {
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                other => break other,
            }
        };
        let n = read.map_err(|e| {
            self.buf.truncate(len);
            e.to_string()
        })?;
        self.buf.truncate(len + n);
        self.done = n == 0;
        Ok(n > 0)
    }

    /// Where `needle` next starts, at or after `from`, reading on as needed.
    fn find(&mut self, mut from: usize, needle: &[u8]) -> Result<Option<usize>, String> {
        loop {
            let hay = &self.buf[self.pos..];
            if let Some(i) = find_bytes(hay.get(from..).unwrap_or_default(), needle) {
                return Ok(Some(from + i));
            }
            from = from.max((hay.len() + 1).saturating_sub(needle.len()));
            if !self.more()? {
                return Ok(None);
            }
        }
    }

    /// Where the tag starting at `pos` closes with `>`, skipping any inside quoted values.
    fn tag_end(&mut self) -> Result<usize, String> {
        let (mut at, mut quote) = (1, None);
        loop {
            let hay = &self.buf[self.pos..];
            while at < hay.len() {
                match (hay[at], quote) {
                    (b'>', None) => return Ok(at),
                    (q @ (b'"' | b'\''), None) => quote = Some(q),
                    (c, Some(q)) if c == q => quote = None,
                    _ => {}
                }
                at += 1;
            }
            if !self.more()? {
                return Err("the guide ends inside a tag".into());
            }
        }
    }
}

fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    let mut from = 0;
    while let Some(i) = hay[from..].iter().position(|&b| b == needle[0]) {
        let at = from + i;
        if hay[at..].starts_with(needle) {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

/// Whether `head` starts the element `name`, not merely one whose name begins with it.
fn is_tag(head: &[u8], name: &[u8]) -> bool {
    head.get(1..=name.len()) == Some(name) && head.get(name.len() + 1).is_some_and(|c| c.is_ascii_whitespace() || matches!(c, b'>' | b'/'))
}

/// The attributes of a start tag given without its closing `>`, as raw names and values.
struct Attributes<'a>(&'a [u8]);

impl<'a> Iterator for Attributes<'a> {
    type Item = (&'a [u8], &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        let s = self.0;
        let skip_space = |mut i: usize| {
            while i < s.len() && s[i].is_ascii_whitespace() {
                i += 1;
            }
            i
        };
        // The element's name comes first and is passed over like an attribute without a value.
        let mut i = 0;
        loop {
            i = skip_space(i);
            let name_start = i;
            while i < s.len() && !s[i].is_ascii_whitespace() && !matches!(s[i], b'=' | b'/' | b'<') {
                i += 1;
            }
            let name = &s[name_start..i];
            i = skip_space(i);
            if i >= s.len() {
                self.0 = &[];
                return None;
            }
            if s[i] != b'=' {
                i += usize::from(i == name_start);
                continue;
            }
            i = skip_space(i + 1);
            let (value, rest) = match s.get(i) {
                Some(&q @ (b'"' | b'\'')) => {
                    let end = s[i + 1..].iter().position(|&c| c == q).map_or(s.len(), |e| i + 1 + e);
                    (&s[i + 1..end], (end + 1).min(s.len()))
                }
                _ => {
                    let end = s[i..].iter().position(u8::is_ascii_whitespace).map_or(s.len(), |e| i + e);
                    (&s[i..end], end)
                }
            };
            self.0 = &s[rest..];
            return Some((name, value));
        }
    }
}

/// The child elements of `element` from `from` on, as names and raw contents. Self-closing
/// children have empty contents.
struct Children<'a>(&'a [u8], usize);

impl<'a> Iterator for Children<'a> {
    type Item = (&'a [u8], &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        let el = self.0;
        loop {
            let at = self.1 + el.get(self.1..)?.iter().position(|&b| b == b'<')?;
            let head = &el[at..];
            if let Some(skip) = [(&b"<!--"[..], &b"-->"[..]), (b"<![CDATA[", b"]]>")].iter().find(|(open, _)| head.starts_with(open)).map(|(_, close)| close) {
                self.1 = find_bytes(&el[at..], skip).map_or(el.len(), |e| at + e + skip.len());
                continue;
            }
            if matches!(head.get(1), Some(b'/' | b'!' | b'?')) {
                self.1 = at + 1;
                continue;
            }
            let name_end = head.iter().position(|c| c.is_ascii_whitespace() || matches!(c, b'>' | b'/')).unwrap_or(head.len());
            let name = &head[1..name_end];
            let open_end = at + tag_close(head)?;
            if el[open_end - 1] == b'/' {
                self.1 = open_end + 1;
                return Some((name, &[]));
            }
            let mut close = Vec::with_capacity(name.len() + 2);
            close.extend_from_slice(b"</");
            close.extend_from_slice(name);
            let content_start = open_end + 1;
            let content_end = find_bytes(&el[content_start..], &close).map_or(el.len(), |e| content_start + e);
            self.1 = content_end + el[content_end..].iter().position(|&b| b == b'>').map_or(el.len() - content_end, |e| e + 1);
            return Some((name, &el[content_start..content_end]));
        }
    }
}

/// Where the tag at the start of `head` closes with `>`, skipping any inside quoted values.
fn tag_close(head: &[u8]) -> Option<usize> {
    let mut quote = None;
    for (i, &c) in head.iter().enumerate().skip(1) {
        match (c, quote) {
            (b'>', None) => return Some(i),
            (b'"' | b'\'', None) => quote = Some(c),
            (c, Some(q)) if c == q => quote = None,
            _ => {}
        }
    }
    None
}

/// Up to about `max` bytes of text content, with entities and CDATA sections decoded and any
/// markup inside dropped. Bytes that are not UTF-8 become replacement characters.
fn decode(raw: &[u8], max: usize) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < raw.len() && out.len() < max {
        let next = raw[i..].iter().position(|&b| b == b'<' || b == b'&').map_or(raw.len(), |n| i + n).min(i + max);
        out.push_str(&String::from_utf8_lossy(&raw[i..next]));
        i = next;
        let rest = &raw[i.min(raw.len())..];
        if rest.starts_with(b"<![CDATA[") {
            let end = find_bytes(&rest[9..], b"]]>").map_or(rest.len(), |e| 9 + e);
            out.push_str(&String::from_utf8_lossy(&rest[9..end]));
            i += (end + 3).min(rest.len());
        } else if rest.starts_with(b"<") {
            i += tag_close(rest).map_or(rest.len(), |e| e + 1);
        } else if rest.starts_with(b"&") {
            let end = rest.iter().take(12).position(|&b| b == b';');
            match end.and_then(|e| entity(&rest[1..e])) {
                Some(c) => {
                    out.push(c);
                    i += end.unwrap_or(0) + 1;
                }
                None => {
                    out.push('&');
                    i += 1;
                }
            }
        }
    }
    out
}

/// The character an entity stands for: `amp`, `#8217`, `#x2019` and friends.
fn entity(name: &[u8]) -> Option<char> {
    Some(match name {
        b"amp" => '&',
        b"lt" => '<',
        b"gt" => '>',
        b"quot" => '"',
        b"apos" => '\'',
        b"nbsp" => ' ',
        [b'#', b'x' | b'X', hex @ ..] => char::from_u32(u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?)?,
        [b'#', dec @ ..] => char::from_u32(std::str::from_utf8(dec).ok()?.parse().ok()?)?,
        _ => return None,
    })
}

/// FNV-1a: far cheaper than the default hasher inside the sandbox, and guide ids need no
/// defence against collisions.
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl std::hash::Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }
}

type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<Fnv>>;

fn clean(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// "20261019001500 +0000", "20261019011500 +01:00" or "202610190015" → epoch ms. A missing or
/// named zone means UTC.
pub fn parse_xmltv_time(s: &str) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits < 12 {
        return None;
    }
    let num = |at: usize| u32::from(b[at] - b'0') * 10 + u32::from(b[at + 1] - b'0');
    let year = i64::from(num(0) * 100 + num(2));
    let (month, day, hour, minute) = (num(4), num(6), num(8), num(10));
    let second = if digits >= 14 { num(12) } else { 0 };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 24 || minute > 59 || second > 60 {
        return None;
    }
    let zone = s[digits..].trim_start().as_bytes();
    let offset = match zone {
        [sign @ (b'+' | b'-'), rest @ ..] => {
            let mut d = rest.iter().filter(|&&c| c != b':').take_while(|c| c.is_ascii_digit()).map(|c| i32::from(c - b'0'));
            let (h1, h2, m1, m2) = (d.next(), d.next(), d.next(), d.next());
            let v = match (h1, h2, m1, m2, d.next()) {
                (Some(h1), Some(h2), None, None, None) => (h1 * 10 + h2) * 60,
                (Some(h1), Some(h2), Some(m1), Some(m2), None) => (h1 * 10 + h2) * 60 + m1 * 10 + m2,
                _ => return None,
            };
            if *sign == b'-' { -v } else { v }
        }
        _ => 0,
    };
    Some(epoch_ms(year, month, day, hour, minute, second, offset))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const HOUR: i64 = 3_600_000;

    fn channel(id: &str, name: &str) -> Channel {
        Channel { id: id.into(), tvg_id: Some(id.into()), name: name.into(), tvg_name: None, number: None, logo: None, group: None, url: format!("http://s/{id}") }
    }

    fn at(stamp: &str) -> i64 {
        parse_xmltv_time(stamp).unwrap()
    }

    fn read(xml: &str, channels: &[Channel], window: (i64, i64), budget: usize) -> Vec<Item> {
        let mut guide = Guide::new(channels, window, budget);
        read_guide(xml.as_bytes(), &mut guide, MAX_GUIDE_BYTES).unwrap();
        guide.into_items()
    }

    #[test]
    fn a_programme_without_a_stop_ends_where_the_next_one_starts() {
        let xml = r#"<tv>
<programme start="20260101180000 +0000" channel="abc"><title>News</title></programme>
<programme start="20260101190000 +0000" channel="abc"><title>Quiz</title></programme>
<programme start="20260101193000 +0000" channel="abc"><title>Late</title></programme>
<programme start="20260101120000 +0000" channel="abc"><title>Over</title></programme>
<programme start="20260101150000 +0000" channel="abc"><title>Afternoon</title></programme>
</tv>"#;
        let items = read(xml, &[channel("abc", "ABC")], (at("20260101170000 +0000"), at("20260102050000 +0000")), 100);
        let got: Vec<(&str, i64, i64)> = items.iter().map(|i| (i.title.as_str(), i.start_ms.unwrap(), i.end_ms.unwrap())).collect();
        assert_eq!(
            got,
            vec![
                ("Afternoon", at("20260101150000 +0000"), at("20260101180000 +0000")),
                ("News", at("20260101180000 +0000"), at("20260101190000 +0000")),
                ("Quiz", at("20260101190000 +0000"), at("20260101193000 +0000")),
            ]
        );
    }

    #[test]
    fn parses_extended_m3u() {
        let m3u = r#"#EXTM3U
#EXTINF:-1 tvg-id="abc.au" tvg-name="ABC" tvg-logo="https://x/abc.png" group-title="News",ABC TV
https://example.com/abc.m3u8
#EXTINF:-1 group-title="Sport",Sport One
https://example.com/sport.m3u8
#EXTINF:-1 tvg-chno="7",Seven
https://example.com/seven.m3u8
"#;
        let (chans, total) = parse_m3u(m3u, None, 100);
        assert_eq!((chans.len(), total), (3, 3));
        assert_eq!(chans[0].id, "abc.au");
        assert_eq!(chans[0].logo.as_deref(), Some("https://x/abc.png"));
        assert_eq!(chans[0].number.as_deref(), Some("1"));
        assert_eq!(chans[1].id, "sport-one");
        assert_eq!(chans[1].tvg_id, None);
        assert_eq!(chans[2].number.as_deref(), Some("7"));
        assert_eq!(parse_m3u(m3u, Some("sport"), 100).0.len(), 1);
        let (first, total) = parse_m3u(m3u, None, 2);
        assert_eq!((first.len(), total), (2, 3));
    }

    #[test]
    fn titles_run_from_the_first_comma_after_the_attributes() {
        let m3u = r#"#EXTINF:-1 tvg-id="sky1" group-title="Sport, UK",Sky Sports 1, HD
http://s/1
#EXTINF:-1,Plain, with a comma
http://s/2
#EXTINF:-1 tvg-name="Fallback name" group-title="x",
http://s/3
"#;
        let (chans, _) = parse_m3u(m3u, None, 10);
        assert_eq!(chans[0].name, "Sky Sports 1, HD");
        assert_eq!(chans[0].group.as_deref(), Some("Sport, UK"));
        assert_eq!(chans[1].name, "Plain, with a comma");
        assert_eq!(chans[2].name, "Fallback name");
        assert_eq!(parse_m3u(m3u, Some("sport, uk"), 10).0.len(), 1);
    }

    #[test]
    fn odd_playlists_still_parse() {
        let m3u = "\u{feff}#EXTINF:-1\ttvg-id='one.uk' group-title='News, UK',One\nhttp://s/1\n\
            #EXTINF:-1 tvg-id=\"two\",Two\nhttp://s/2\n";
        let (channels, _) = parse_m3u(m3u, Some("News, UK"), 10);
        assert_eq!(channels.len(), 1);
        assert_eq!(channels[0].tvg_id.as_deref(), Some("one.uk"));
        assert_eq!(channels[0].name, "One");
    }

    #[test]
    fn films_and_series_are_not_channels() {
        let m3u = "#EXTM3U\n\
            #EXTINF:-1 tvg-id=\"one\" group-title=\"News\",One\nhttp://x.tv:8080/live/u/p/1.ts\n\
            #EXTINF:-1 group-title=\"VOD | Drama\",A Film (2019)\nhttp://x.tv:8080/movie/u/p/77.mkv\n\
            #EXTINF:-1 group-title=\"Series\",A Show S01 E01\nhttp://x.tv:8080/series/u/p/78.mp4\n\
            #EXTINF:-1 group-title=\"News\",Two\nhttp://x.tv:8080/u/p/2\n";
        let (channels, total) = parse_m3u(m3u, None, 100);
        assert_eq!(channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["One", "Two"]);
        assert_eq!(total, 2);
        assert_eq!(channels[1].number.as_deref(), Some("2"));
    }

    #[test]
    fn channel_ids_are_unique_and_stable() {
        let m3u = r#"#EXTM3U
#EXTINF:-1 tvg-id="bbc1.uk" group-title="UK",BBC One
http://s/bbc1
#EXTINF:-1 tvg-id="bbc1.uk" group-title="HD",BBC One HD
http://s/bbc1hd
#EXTINF:-1 tvg-id="bbc1.uk~2" group-title="HD",Imposter
http://s/odd
#EXTINF:-1,!!!
http://s/a
#EXTINF:-1,???
http://s/b
#EXTINF:-1,★
http://s/a
#EXTINF:-1,News
http://s/n1
#EXTINF:-1,News
http://s/n2
"#;
        let (chans, _) = parse_m3u(m3u, None, 100);
        let ids: Vec<&str> = chans.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(&ids[..3], ["bbc1.uk", "bbc1.uk~2", "bbc1.uk~2~2"]);
        assert!(ids[3].starts_with("url-") && ids[4].starts_with("url-") && ids[3] != ids[4]);
        assert_eq!(ids[5], format!("{}~2", ids[3]));
        assert_eq!(&ids[6..], ["news", "news~2"]);
        assert_eq!(ids.iter().collect::<HashSet<_>>().len(), ids.len());
        assert_eq!(chans[1].tvg_id.as_deref(), Some("bbc1.uk"));
        let again: Vec<String> = parse_m3u(m3u, None, 100).0.into_iter().map(|c| c.id).collect();
        assert_eq!(ids, again);
        let hd: Vec<String> = parse_m3u(m3u, Some("HD"), 100).0.into_iter().map(|c| c.id).collect();
        assert_eq!(hd, ["bbc1.uk~2", "bbc1.uk~2~2"]);
        let items: Vec<Item> = chans.iter().map(Channel::to_item).collect();
        assert_eq!(items.iter().map(|i| &i.id).collect::<HashSet<_>>().len(), items.len());
    }

    #[test]
    fn xmltv_times_take_every_offset_style() {
        let utc = 1_792_368_900_000;
        assert_eq!(parse_xmltv_time("20261019001500 +0000"), Some(utc));
        assert_eq!(parse_xmltv_time("20261019101500 +1000"), Some(utc));
        assert_eq!(parse_xmltv_time("20261019011500 +01:00"), Some(utc));
        assert_eq!(parse_xmltv_time("20261019011500+01:00"), Some(utc));
        assert_eq!(parse_xmltv_time("20261018191500 -05:00"), Some(utc));
        assert_eq!(parse_xmltv_time("20261019011500 +01"), Some(utc));
        assert_eq!(parse_xmltv_time("20261019001500"), Some(utc));
        assert_eq!(parse_xmltv_time("202610190015"), Some(utc));
        assert_eq!(parse_xmltv_time("20261019001500 UTC"), Some(utc));
        for bad in ["20261019001500 +1é00", "20261019001500 +", "2026101900", "20261319001500", "２０２６1019001500", ""] {
            assert_eq!(parse_xmltv_time(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn programmes_keep_entities_cdata_and_prefer_desc() {
        let abc = channel("abc.au", "ABC TV");
        let xml = r#"<?xml version="1.0"?><tv>
<channel id="abc.au"><display-name>ABC</display-name></channel>
<programme start="20261019000000 +0000" stop="20261019003000 +0000" channel="abc.au">
  <title lang="en">Law &amp; Order</title><title lang="fr">New York, police judiciaire</title>
  <sub-title>The &#8220;Pilot&#8221;</sub-title>
  <desc>Detectives &amp; <![CDATA[lawyers <in> court]]> take   on
     a case.</desc>
</programme>
<programme start="20261019003000 +01:00" stop="20261019010000 +0000" channel="abc.au"><title>Q&amp;A</title><sub-title>Only a sub-title</sub-title></programme>
<programme start="20261019010000 +0000" stop="20261019013000 +0000" channel="abc.au"><title><![CDATA[Tom & Jerry]]></title></programme>
<programme start="20261019013000 +0000" stop="20261019020000 +0000" channel="abc.au"><title>AT&T Presents</title></programme>
</tv>"#;
        let items = read(xml, std::slice::from_ref(&abc), (at("20261018230000"), at("20261019060000")), 100);
        let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["Q&A", "Law & Order", "Tom & Jerry", "AT&T Presents"]);
        assert_eq!(items[0].subtitle.as_deref(), Some("Only a sub-title"));
        assert_eq!(items[0].start_ms, Some(at("20261018233000")));
        assert_eq!(items[1].subtitle.as_deref(), Some("Detectives & lawyers <in> court take on a case."));
        assert_eq!(items[1].id, format!("prog:abc.au:{}", at("20261019000000")));
        assert_eq!(items[1].channel_id.as_deref(), Some("abc.au"));
        assert!(items[1].action.is_empty());
    }

    #[test]
    fn only_programmes_in_the_window_are_kept() {
        let abc = channel("abc.au", "ABC TV");
        let xml = r#"<tv>
<programme start="20261019000000 +0000" stop="20261019003000 +0000" channel="abc.au"><title>Over</title></programme>
<programme start="20261019003000 +0000" stop="20261019010000 +0000" channel="abc.au"><title>On now</title></programme>
<programme start="20261019010000 +0000" stop="20261019013000 +0000" channel="abc.au"><title>Next</title></programme>
<programme start="20261019070000 +0000" stop="20261019073000 +0000" channel="abc.au"><title>Tomorrow</title></programme>
<programme start="20261019013000 +0000" stop="20261019020000 +0000" channel="other"><title>Elsewhere</title></programme>
<programme start="20261019013000 +0000" channel="abc.au"><title>No stop</title></programme>
</tv>"#;
        let items = read(xml, std::slice::from_ref(&abc), (at("20261019004500"), at("20261019064500")), 100);
        assert_eq!(items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["On now", "Next"]);
    }

    #[test]
    fn guide_ids_that_differ_are_matched_by_display_name() {
        let abc = channel("abc.au", "ABC TV");
        let xml = r#"<tv><channel id="guide-7"><display-name>ABC TV (1080p)</display-name></channel>
<programme start="20261019000000 +0000" stop="20261019003000 +0000" channel="guide-7"><title>Matched</title></programme></tv>"#;
        let channels = [abc];
        let mut guide = Guide::new(&channels, (at("20261018230000"), at("20261019060000")), 100);
        read_guide(xml.as_bytes(), &mut guide, MAX_GUIDE_BYTES).unwrap();
        assert_eq!(guide.aliases.len(), 1);
        let items = guide.into_items();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].channel_id.as_deref(), Some("abc.au"));
    }

    #[test]
    fn the_guide_finds_a_channel_by_tvg_name_or_its_id_in_another_case() {
        let m3u = "#EXTINF:-1 tvg-name=\"BBC One\",UK: BBC One HD\nhttp://s/1\n#EXTINF:-1 tvg-id=\"ITV1.uk\",ITV\nhttp://s/2\n";
        let (chans, _) = parse_m3u(m3u, None, 10);
        let xml = r#"<tv><channel id="bbc1"><display-name>BBC One</display-name></channel><channel id="itv1.uk"><display-name>ITV1</display-name></channel>
<programme start="20261019000000 +0000" stop="20261019003000 +0000" channel="bbc1"><title>News</title></programme>
<programme start="20261019000000 +0000" stop="20261019003000 +0000" channel="itv1.uk"><title>Quiz</title></programme></tv>"#;
        let mut guide = Guide::new(&chans, (at("20261018230000"), at("20261019060000")), 100);
        read_guide(xml.as_bytes(), &mut guide, MAX_GUIDE_BYTES).unwrap();
        let titles: Vec<String> = guide.into_items().into_iter().map(|i| i.title).collect();
        assert_eq!(titles, ["News", "Quiz"]);
    }

    #[test]
    fn channels_sharing_a_tvg_id_both_get_the_guide() {
        let m3u = "#EXTINF:-1 tvg-id=\"abc.au\",ABC\nhttp://s/sd\n#EXTINF:-1 tvg-id=\"abc.au\",ABC HD\nhttp://s/hd\n";
        let (chans, _) = parse_m3u(m3u, None, 10);
        let xml = r#"<tv><programme start="20261019000000 +0000" stop="20261019003000 +0000" channel="abc.au"><title>News</title></programme>
<programme start="20261019000000 +0000" stop="20261019003000 +0000" channel="abc.au"><title>News again</title></programme></tv>"#;
        let items = read(xml, &chans, (at("20261018230000"), at("20261019060000")), 100);
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        let start = at("20261019000000");
        assert_eq!(ids, [format!("prog:abc.au:{start}"), format!("prog:abc.au~2:{start}")]);
        assert_eq!(items[0].title, "News");
    }

    /// A guide written channel by channel, as most are, with far more than fits.
    fn big_guide(channels: usize, per_channel: usize) -> String {
        let mut xml = String::from("<tv>\n");
        let base = at("20261019000000");
        for c in 0..channels {
            for p in 0..per_channel {
                let start = base + p as i64 * HOUR / 2;
                let stamp = |ms: i64| {
                    let mins = (ms - base) / 60_000;
                    format!("20261019{:02}{:02}00 +0000", mins / 60, mins % 60)
                };
                xml.push_str(&format!("<programme start=\"{}\" stop=\"{}\" channel=\"c{c}\"><title>P{p}</title><desc>About programme {p}.</desc></programme>\n", stamp(start), stamp(start + HOUR / 2)));
            }
        }
        xml.push_str("</tv>\n");
        xml
    }

    #[test]
    fn the_budget_is_spread_over_every_channel_by_start_time() {
        let channels: Vec<Channel> = (0..50).map(|c| channel(&format!("c{c}"), &format!("Channel {c}"))).collect();
        let xml = big_guide(50, 40);
        let items = read(&xml, &channels, (at("20261019000000"), at("20261019235900")), 120);
        assert_eq!(items.len(), 120);
        let expected = [at("20261019000000"), at("20261019003000"), at("20261019010000")];
        for c in &channels {
            let starts: Vec<i64> = items.iter().filter(|i| i.channel_id.as_deref() == Some(&c.id)).map(|i| i.start_ms.unwrap()).collect();
            assert!(starts == expected[..2] || starts == expected, "{}: {starts:?}", c.id);
        }
        let mut uneven = Guide::new(&channels, (at("20261019000000"), at("20261019235900")), 1000);
        read_guide(xml.as_bytes(), &mut uneven, MAX_GUIDE_BYTES).unwrap();
        assert!(uneven.stored <= 2 * 1000 + 50 + 256);
        assert_eq!(uneven.into_items().len(), 1000);
    }

    #[test]
    fn gzipped_guides_are_read_as_they_unpack() {
        let channels: Vec<Channel> = (0..5).map(|c| channel(&format!("c{c}"), &format!("Channel {c}"))).collect();
        let xml = big_guide(5, 40);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(xml.as_bytes()).unwrap();
        let gz = gz.finish().unwrap();
        let window = (at("20261019000000"), at("20261019235900"));
        let mut guide = Guide::new(&channels, window, 1000);
        read_guide(&gz, &mut guide, MAX_GUIDE_BYTES).unwrap();
        assert_eq!(guide.into_items().len(), 200);

        let mut capped = Guide::new(&channels, window, 1000);
        let err = read_guide(&gz, &mut capped, xml.len() / 2).unwrap_err();
        assert!(err.contains("unpacks to more than"), "{err}");
        let kept = capped.into_items().len();
        assert!(kept > 50 && kept < 200, "{kept}");

        let mut broken = Guide::new(&channels, window, 1000);
        assert!(read_guide(&gz[..gz.len() / 2], &mut broken, MAX_GUIDE_BYTES).is_err());
        assert!(!broken.into_items().is_empty());
    }

    #[test]
    fn reading_stops_at_the_deadline_with_what_came_before() {
        let channels: Vec<Channel> = (0..50).map(|c| channel(&format!("c{c}"), &format!("Channel {c}"))).collect();
        let mut guide = Guide::new(&channels, (at("20261019000000"), at("20261019235900")), 500);
        guide.deadline_ms = -1;
        let err = read_guide(big_guide(50, 40).as_bytes(), &mut guide, MAX_GUIDE_BYTES).unwrap_err();
        assert!(err.contains("seconds"), "{err}");
        assert_eq!(guide.seen, 1023);
        assert!(!guide.into_items().is_empty());
    }

    #[test]
    fn a_malformed_guide_keeps_what_came_before_the_fault() {
        let abc = channel("abc.au", "ABC TV");
        let xml = r#"<tv><programme start="20261019000000 +0000" stop="20261019003000 +0000" channel="abc.au"><title>Fine</title></programme>
<programme start="20261019003000 +0000" stop="20261019010000 +0000" channel="abc.au"><title>Cut off</title></programme><!-- never closed"#;
        let channels = [abc];
        let mut guide = Guide::new(&channels, (at("20261018230000"), at("20261019060000")), 100);
        assert!(read_guide(xml.as_bytes(), &mut guide, MAX_GUIDE_BYTES).is_err());
        assert_eq!(guide.into_items().iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["Fine", "Cut off"]);
    }
}
