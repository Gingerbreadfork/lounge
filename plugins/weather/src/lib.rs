//! Weather: how it is outside now and the week ahead for one town, as a row of small tiles to
//! read. The forecast and the town's whereabouts come from Open-Meteo, which asks for no key or
//! account, and the symbols are those of Yr, the Norwegian weather service.
use lounge_plugin_sdk::{self as sdk, host, HttpRequest, Item, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use serde::{Deserialize, Serialize};

pub struct Weather;

const SERVICE: &str = "Open-Meteo";
const FORECAST: &str = "https://api.open-meteo.com/v1/forecast";
const GEOCODING: &str = "https://geocoding-api.open-meteo.com/v1/search";
const PLACE_BY_ID: &str = "https://geocoding-api.open-meteo.com/v1/get";
/// Yr's weather symbols, at one commit so a picture never changes under its address.
const SYMBOLS: &str = "https://cdn.jsdelivr.net/gh/metno/weathericons@89e3173756248b4696b9b10677b66c4ef435db53/weather/png";
const ROW: &str = "forecast";
const PLACE_KEY: &str = "place";
const FORECAST_KEY: &str = "forecast";
const DAYS: i64 = 7;
const DAY_MS: i64 = 86_400_000;
/// How long a reading is shown as how it is now.
const NOW_LASTS_MS: i64 = 60 * 60_000;
/// A town of this many people is taken as the one meant without asking in another language.
const KNOWN_TOWN: f64 = 1_000.0;
/// A place this large ends the search through the languages.
const CITY: f64 = 100_000.0;
/// Asked after English for a name in Latin letters. Open-Meteo matches a town only under the
/// name it has in the language asked for, so Wien is found in German and Lisboa in Portuguese.
const LATIN: [&str; 16] = ["de", "fr", "es", "it", "pt", "nl", "da", "sv", "no", "fi", "pl", "cs", "hu", "ro", "tr", "vi"];

impl sdk::Plugin for Weather {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.weather".into(),
            name: "Weather".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Now and the week ahead for the town you name, with weather data by Open-Meteo.com.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content],
            fetch_hosts: vec!["api.open-meteo.com".into(), "geocoding-api.open-meteo.com".into(), "cdn.jsdelivr.net".into()],
            refresh_minutes: 30,
            rows: vec![RowSpec { id: ROW.into(), title: "Weather".into(), style: RowStyle::Compact }],
            settings: vec![
                SettingSpec {
                    key: "place".into(),
                    label: "Town or city".into(),
                    kind: SettingKind::Text,
                    required: true,
                    help: Some("Add the region after a comma when the name is shared, as in Portland, Maine. Coordinates work too, as in 45.76, 4.84.".into()),
                    default: None,
                },
                SettingSpec { key: "fahrenheit".into(), label: "Temperatures in Fahrenheit".into(), kind: SettingKind::Toggle, required: false, help: None, default: Some("false".into()) },
            ],
            homepage: Some("https://open-meteo.com".into()),
            self_signed_servers: false,
            for_apps: Vec::new(),
            sent_to: Default::default(),
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let place = locate(&host::required_setting("place", "Town or city")?)?;
        let fahrenheit = host::setting("fahrenheit").is_some_and(|v| v == "true");
        let url = forecast_url(&place, fahrenheit);
        match get_json::<Forecast>(&url) {
            Ok(forecast) => {
                let items = tiles(&forecast, &place, fahrenheit, &Moment::fresh(&forecast, ctx.now_ms));
                if !items.iter().any(|item| item.id.starts_with("day:")) {
                    sdk::warn!("the forecast held no day with a temperature");
                    return Err(sdk::unreadable(SERVICE, &url));
                }
                sdk::info!("{}", summary(&place, &forecast.current, fahrenheit));
                let kept = Kept { asked: place.asked.clone(), fahrenheit, fetched_ms: ctx.now_ms, forecast };
                if let Ok(json) = serde_json::to_vec(&kept) {
                    host::kv_set(FORECAST_KEY, &json);
                }
                Ok(items)
            }
            // While the network is away the last forecast still says what the coming days hold.
            Err(Failure::Passing(why)) => {
                let kept = host::kv_get(FORECAST_KEY).and_then(|v| serde_json::from_slice::<Kept>(&v).ok());
                let Some(kept) = kept.filter(|k| k.asked == place.asked && k.fahrenheit == fahrenheit) else { return Err(why) };
                let age = ctx.now_ms - kept.fetched_ms;
                let items = tiles(&kept.forecast, &place, fahrenheit, &Moment::later(&kept.forecast, ctx.now_ms, age));
                sdk::warn!("{why}");
                if !items.iter().any(|item| item.id.starts_with("day:")) {
                    // Every day of it has passed, so the row is put away and the next failure shows.
                    host::kv_delete(FORECAST_KEY);
                    return Ok(Vec::new());
                }
                sdk::info!("Showing the forecast from {} ago", span(age));
                Ok(items)
            }
            Err(Failure::Lasting(why)) => Err(why),
        }
    }
}

sdk::export_plugin!(Weather);

/// Why a request failed: for now, as when the network is away, or until something is changed.
#[derive(Debug, PartialEq)]
enum Failure {
    Passing(String),
    Lasting(String),
}

impl From<Failure> for String {
    fn from(failure: Failure) -> String {
        match failure {
            Failure::Passing(why) | Failure::Lasting(why) => why,
        }
    }
}

/// A busy or broken server may be well again by the next refresh; a refused request will not be.
fn failed_with(status: u16) -> Failure {
    let why = sdk::online_error(SERVICE, status);
    if status == 429 || status >= 500 { Failure::Passing(why) } else { Failure::Lasting(why) }
}

/// Where the forecast is for, kept between refreshes so a town is looked up once.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Place {
    /// The setting's value this was found for.
    asked: String,
    /// "Portland", or nothing for coordinates typed in.
    town: Option<String>,
    /// "Oregon, United States".
    region: Option<String>,
    latitude: f64,
    longitude: f64,
}

impl Place {
    /// "Portland, Oregon, United States", or the coordinates to the kilometre.
    fn name(&self) -> String {
        match &self.town {
            Some(town) => [Some(town), self.region.as_ref()].into_iter().flatten().cloned().collect::<Vec<_>>().join(", "),
            None => format!("{:.2}, {:.2}", self.latitude, self.longitude),
        }
    }
}

#[derive(Deserialize, Default)]
struct Geocoding {
    #[serde(default)]
    results: Vec<Found>,
}

#[derive(Deserialize)]
struct Found {
    #[serde(default)]
    id: Option<i64>,
    name: String,
    latitude: f64,
    longitude: f64,
    #[serde(default)]
    admin1: Option<String>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    population: Option<f64>,
    /// GeoNames' kind of place. Towns and cities start with "PPL".
    #[serde(default)]
    feature_code: Option<String>,
}

impl Found {
    /// How many people live there. A country, a mountain or a town of unknown size counts none.
    fn size(&self) -> f64 {
        let town = self.feature_code.as_deref().is_some_and(|code| code.starts_with("PPL"));
        if town { self.population.unwrap_or(0.0) } else { 0.0 }
    }

    fn into_place(self, asked: &str) -> Place {
        // A city state is its own region and country, and says so once.
        let mut around: Vec<String> = [self.admin1, self.country].into_iter().flatten().filter(|part| !part.trim().is_empty() && *part != self.name).collect();
        around.dedup();
        Place { asked: asked.to_string(), town: Some(self.name), region: (!around.is_empty()).then(|| around.join(", ")), latitude: self.latitude, longitude: self.longitude }
    }
}

/// The size of the first place found, with nothing found below a place of no size.
fn size(found: &[Found]) -> f64 {
    found.first().map_or(-1.0, Found::size)
}

/// One answer from the forecast API. The offset is the place's own, in seconds east of UTC.
#[derive(Serialize, Deserialize)]
struct Forecast {
    #[serde(default)]
    utc_offset_seconds: i64,
    current: Current,
    #[serde(default)]
    daily: Daily,
}

#[derive(Serialize, Deserialize)]
struct Current {
    temperature_2m: f64,
    #[serde(default)]
    is_day: Option<f64>,
    #[serde(default)]
    weather_code: Option<f64>,
}

/// One list per fact, a day to each place. A day the model does not reach is `null`.
#[derive(Serialize, Deserialize, Default)]
struct Daily {
    #[serde(default)]
    time: Vec<String>,
    #[serde(default)]
    weather_code: Vec<Option<f64>>,
    #[serde(default)]
    temperature_2m_max: Vec<Option<f64>>,
    /// Seconds of daylight, which is none at all in a polar winter.
    #[serde(default)]
    daylight_duration: Vec<Option<f64>>,
}

/// The last forecast that came through, to show its days while the network is away.
#[derive(Serialize, Deserialize)]
struct Kept {
    asked: String,
    fahrenheit: bool,
    fetched_ms: i64,
    forecast: Forecast,
}

/// Why Open-Meteo refused a request, in its own words.
#[derive(Deserialize)]
struct Refusal {
    reason: String,
}

fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, Failure> {
    let response = sdk::fetch_online(SERVICE, &HttpRequest::get(url)).map_err(Failure::Passing)?;
    if !response.ok() {
        if let Ok(refusal) = serde_json::from_str::<Refusal>(&response.body) {
            sdk::warn!("{}", refusal.reason);
        }
        return Err(failed_with(response.status));
    }
    serde_json::from_str(&response.body).map_err(|e| {
        sdk::warn!("{e}");
        Failure::Lasting(sdk::unreadable(SERVICE, url))
    })
}

/// The place the setting names: coordinates as typed, or a town looked up by name and remembered.
fn locate(asked: &str) -> Result<Place, String> {
    let asked = asked.trim();
    if let Some((latitude, longitude)) = coordinates(asked) {
        return Ok(Place { asked: asked.to_string(), town: None, region: None, latitude, longitude });
    }
    let known = host::kv_get(PLACE_KEY).and_then(|v| serde_json::from_slice::<Place>(&v).ok());
    if let Some(place) = known.filter(|p| p.asked == asked) {
        return Ok(place);
    }
    let (found, language) = find(asked)?;
    let found = found.into_iter().next().ok_or_else(|| unknown_place(asked))?;
    // The row is in English, so a town found under another language's name is named in English.
    let named = if language == "en" { None } else { in_english(&found) };
    let place = named.unwrap_or(found).into_place(asked);
    if let Ok(json) = serde_json::to_vec(&place) {
        host::kv_set(PLACE_KEY, &json);
    }
    Ok(place)
}

/// What Open-Meteo finds for a name, and the language it was found in. A name that is nothing
/// much in English is asked for in other languages too, and the largest town any of them knows
/// by it is taken.
fn find(asked: &str) -> Result<(Vec<Found>, &'static str), String> {
    let mut best = (search(asked, "en")?, "en");
    if size(&best.0) < KNOWN_TOWN {
        for language in languages_for(asked) {
            let found = search(asked, language)?;
            if size(&found) > size(&best.0) {
                best = (found, language);
            }
            if size(&best.0) >= CITY {
                break;
            }
        }
    }
    Ok(best)
}

/// The same place under its English names, when Open-Meteo will say them.
fn in_english(found: &Found) -> Option<Found> {
    get_json(&format!("{PLACE_BY_ID}?id={}&language=en&format=json", found.id?)).ok()
}

fn search(asked: &str, language: &str) -> Result<Vec<Found>, String> {
    let found: Geocoding = get_json(&format!("{GEOCODING}?name={}&count=1&language={language}&format=json", encode_query(asked)))?;
    Ok(found.results)
}

/// The languages worth asking in for a name, going by the script of its first letter outside
/// plain Latin. A script Open-Meteo has no names in gets none.
fn languages_for(name: &str) -> &'static [&'static str] {
    match name.chars().find(|c| !c.is_ascii() && c.is_alphabetic()).map(u32::from) {
        None | Some(0x00C0..=0x024F | 0x1E00..=0x1EFF) => &LATIN,
        Some(0x0370..=0x03FF) => &["el"],
        Some(0x0400..=0x052F) => &["ru", "uk", "bg"],
        Some(0x0590..=0x05FF) => &["he"],
        Some(0x0600..=0x06FF) => &["ar"],
        Some(0x0E00..=0x0E7F) => &["th"],
        Some(0x3040..=0x30FF | 0x4E00..=0x9FFF) => &["zh", "ja"],
        Some(_) => &[],
    }
}

fn unknown_place(asked: &str) -> String {
    format!("{SERVICE} couldn't find \"{asked}\". Check the spelling, try the name in English, or put a comma before the region, as in Portland, Maine.")
}

/// A latitude and a longitude typed as two numbers. They are told from a postcode such as
/// "040 01" by a comma or semicolon between them, or by a decimal in either.
fn coordinates(text: &str) -> Option<(f64, f64)> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    let (first, second, marked) = match parts[..] {
        [first, second] => (first, second, first.ends_with([',', ';']) || text.contains('.') || (first.contains(',') && second.contains(','))),
        [both] => {
            let (first, second) = both.split_once(';').or_else(|| both.split_once(','))?;
            (first, second, true)
        }
        _ => return None,
    };
    // A comma left inside a number is a decimal comma.
    let number = |part: &str| part.trim_end_matches([',', ';']).replace(',', ".").parse::<f64>().ok();
    let (latitude, longitude) = (number(first)?, number(second)?);
    (marked && (-90.0..=90.0).contains(&latitude) && (-180.0..=180.0).contains(&longitude)).then_some((latitude, longitude))
}

/// The forecast for a place, in its own time zone. Two decimals is about a kilometre, which is all
/// a forecast needs and less than a home's address.
fn forecast_url(place: &Place, fahrenheit: bool) -> String {
    let unit = if fahrenheit { "&temperature_unit=fahrenheit" } else { "" };
    format!(
        "{FORECAST}?latitude={:.2}&longitude={:.2}&current=temperature_2m,is_day,weather_code&daily=weather_code,temperature_2m_max,daylight_duration&timezone=auto&forecast_days={DAYS}{unit}",
        place.latitude, place.longitude
    )
}

fn encode_query(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// A WMO weather code in a tile's words, with the name of the symbol drawn for it. A sky that
/// shows the sun or the moon has a symbol for each, and `lit` says which is up.
fn sky(code: f64, lit: bool) -> Option<(&'static str, String)> {
    let by_light = |name: &str| format!("{name}_{}", if lit { "day" } else { "night" });
    Some(match code as i64 {
        0 => (if lit { "Sunny" } else { "Clear" }, by_light("clearsky")),
        1 => (if lit { "Mostly sunny" } else { "Mostly clear" }, by_light("fair")),
        2 => ("Partly cloudy", by_light("partlycloudy")),
        3 => ("Cloudy", "cloudy".into()),
        45 | 48 => ("Fog", "fog".into()),
        51 => ("Light drizzle", "lightrain".into()),
        53 | 55 => ("Drizzle", "rain".into()),
        56 | 57 => ("Freezing drizzle", "lightsleet".into()),
        61 => ("Light rain", "lightrain".into()),
        63 => ("Rain", "rain".into()),
        65 => ("Heavy rain", "heavyrain".into()),
        66 | 67 => ("Freezing rain", "sleet".into()),
        71 | 77 => ("Light snow", "lightsnow".into()),
        73 => ("Snow", "snow".into()),
        75 => ("Heavy snow", "heavysnow".into()),
        80 => ("Light showers", by_light("lightrainshowers")),
        81 => ("Showers", by_light("rainshowers")),
        82 => ("Heavy showers", by_light("heavyrainshowers")),
        85 => ("Light snow showers", by_light("lightsnowshowers")),
        86 => ("Heavy snow showers", by_light("heavysnowshowers")),
        95 => ("Thunderstorm", "rainandthunder".into()),
        96 | 99 => ("Thunder and hail", "heavyrainandthunder".into()),
        _ => return None,
    })
}

/// A temperature in whole degrees with a true minus sign. Just under zero is "0°", never "−0°".
fn degrees(temperature: f64) -> String {
    let whole = temperature.round() as i64;
    if whole < 0 { format!("\u{2212}{}°", -whole) } else { format!("{whole}°") }
}

/// The reading for now, which alone names its scale, so the row says once which one it is in.
fn reading(now: &Current, fahrenheit: bool) -> String {
    format!("{}{}", degrees(now.temperature_2m), if fahrenheit { "F" } else { "C" })
}

/// When the tiles are made, which decides what a forecast's days are called.
struct Moment {
    /// Today at the place, in days since 1970.
    today: i64,
    /// Whether the reading is recent enough to be how it is now.
    now: bool,
}

impl Moment {
    /// A forecast just fetched starts today, whatever the TV's clock makes of the date.
    fn fresh(forecast: &Forecast, now_ms: i64) -> Moment {
        let today = forecast.daily.time.first().and_then(|date| day_number(date)).unwrap_or_else(|| day_at(forecast, now_ms));
        Moment { today, now: true }
    }

    /// A forecast kept for `age_ms` is named against the clock, and its reading may be too old.
    fn later(forecast: &Forecast, now_ms: i64, age_ms: i64) -> Moment {
        Moment { today: day_at(forecast, now_ms), now: (0..NOW_LASTS_MS).contains(&age_ms) }
    }
}

/// The day it is at the forecast's place at `now_ms`, in days since 1970.
fn day_at(forecast: &Forecast, now_ms: i64) -> i64 {
    (now_ms + forecast.utc_offset_seconds * 1000).div_euclid(DAY_MS)
}

/// The row: where it is for, how it is now, then a tile for each day from today. The ids name
/// places in the row rather than dates, so a tile someone hides stays hidden as the days pass.
fn tiles(forecast: &Forecast, place: &Place, fahrenheit: bool, moment: &Moment) -> Vec<Item> {
    let mut tiles = vec![place_tile(place)];
    if moment.now {
        let now = &forecast.current;
        tiles.push(tile("now".into(), format!("{} now", reading(now, fahrenheit)), sky_now(now)));
    }
    let daily = &forecast.daily;
    for (n, date) in daily.time.iter().enumerate() {
        let at = |list: &[Option<f64>]| list.get(n).copied().flatten();
        let Some(day) = day_number(date) else { continue };
        let (ahead, Some(high)) = (day - moment.today, at(&daily.temperature_2m_max)) else { continue };
        if !(0..DAYS).contains(&ahead) || tiles.iter().any(|tile| tile.id == format!("day:{ahead}")) {
            continue;
        }
        // A day the sun never rises on is clear, not sunny.
        let lit = at(&daily.daylight_duration) != Some(0.0);
        tiles.push(tile(format!("day:{ahead}"), format!("{} {}", degrees(high), day_name(ahead, day)), at(&daily.weather_code).and_then(|code| sky(code, lit))));
    }
    let count = tiles.len();
    for (i, tile) in tiles.iter_mut().enumerate() {
        tile.weight = (count - i) as i32;
    }
    tiles
}

fn sky_now(now: &Current) -> Option<(&'static str, String)> {
    now.weather_code.and_then(|code| sky(code, now.is_day != Some(0.0)))
}

/// The town the forecast is for over its region, so a namesake picked by mistake shows at once.
fn place_tile(place: &Place) -> Item {
    Item { id: "place".into(), title: place.town.clone().unwrap_or_else(|| place.name()), subtitle: place.region.clone(), row: Some(ROW.into()), ..Default::default() }
}

/// One tile: how warm and when, with the sky in words under it and as the symbol beside it. It
/// carries no action, which makes it a tile to read.
fn tile(id: String, title: String, sky: Option<(&'static str, String)>) -> Item {
    let (words, symbol) = sky.unzip();
    Item { id, title, subtitle: words.map(str::to_string), logo: symbol.map(|symbol| format!("{SYMBOLS}/{symbol}.png")), row: Some(ROW.into()), ..Default::default() }
}

/// What a day `ahead` of today is called, as it reads after a temperature.
fn day_name(ahead: i64, day: i64) -> &'static str {
    // The first day of 1970 was a Thursday.
    const DAYS_FROM_THURSDAY: [&str; 7] = ["Thursday", "Friday", "Saturday", "Sunday", "Monday", "Tuesday", "Wednesday"];
    match ahead {
        0 => "today",
        1 => "tomorrow",
        _ => DAYS_FROM_THURSDAY[day.rem_euclid(7) as usize],
    }
}

/// A date such as "2026-11-13" in days since 1970.
fn day_number(date: &str) -> Option<i64> {
    let mut parts = date.trim().splitn(3, '-').map(|p| p.parse::<u32>().ok());
    let (year, month, day) = (parts.next()??, parts.next()??, parts.next()??);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(sdk::epoch_ms(i64::from(year), month, day, 0, 0, 0, 0).div_euclid(DAY_MS))
}

/// The line for the plugin's log: "4°C and light rain in Berlin, State of Berlin, Germany".
fn summary(place: &Place, now: &Current, fahrenheit: bool) -> String {
    let (reading, at) = (reading(now, fahrenheit), place.name());
    match sky_now(now) {
        Some((words, _)) => format!("{reading} and {} in {at}", words.to_lowercase()),
        None => format!("{reading} in {at}"),
    }
}

/// A length of time in the largest unit that fits: "40 minutes", "1 hour", "3 days".
fn span(ms: i64) -> String {
    let minutes = (ms / 60_000).max(1);
    let (count, unit) = match minutes {
        ..60 => (minutes, "minute"),
        60..2_880 => (minutes / 60, "hour"),
        _ => (minutes / 1_440, "day"),
    };
    format!("{count} {unit}{}", if count == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BERLIN: &str = r#"{"latitude":52.512604,"longitude":13.419517,"utc_offset_seconds":3600,"timezone":"Europe/Berlin",
        "current_units":{"time":"iso8601","temperature_2m":"°C"},
        "current":{"time":"2026-11-10T22:15","interval":900,"temperature_2m":3.3,"is_day":0,"weather_code":3},
        "daily":{"time":["2026-11-10","2026-11-11","2026-11-12","2026-11-13","2026-11-14","2026-11-15","2026-11-16"],
            "weather_code":[61,61,3,80,2,2,61],
            "temperature_2m_max":[4.8,4.8,6.4,8.0,7.4,5.8,8.5],
            "daylight_duration":[32400.5,32200.0,32000.0,31800.0,31600.0,31400.0,31200.0]}}"#;

    /// The symbols this plugin may name, each a file at the pinned commit.
    const SERVED: &str = "clearsky_day clearsky_night cloudy fair_day fair_night fog heavyrain heavyrainandthunder heavyrainshowers_day heavyrainshowers_night \
        heavysnow heavysnowshowers_day heavysnowshowers_night lightrain lightrainshowers_day lightrainshowers_night lightsleet lightsnow \
        lightsnowshowers_day lightsnowshowers_night partlycloudy_day partlycloudy_night rain rainandthunder rainshowers_day rainshowers_night sleet snow";

    const CODES: [i64; 28] = [0, 1, 2, 3, 45, 48, 51, 53, 55, 56, 57, 61, 63, 65, 66, 67, 71, 73, 75, 77, 80, 81, 82, 85, 86, 95, 96, 99];

    /// 22:15 in Berlin on the day the forecast above starts.
    const THAT_EVENING_MS: i64 = 1_794_345_300_000;

    fn berlin() -> Place {
        Place { asked: "Berlin".into(), town: Some("Berlin".into()), region: Some("State of Berlin, Germany".into()), latitude: 52.52437, longitude: 13.41053 }
    }

    fn forecast() -> Forecast {
        serde_json::from_str(BERLIN).unwrap()
    }

    fn lines(items: &[Item]) -> Vec<(&str, &str, &str)> {
        items.iter().map(|i| (i.id.as_str(), i.title.as_str(), i.subtitle.as_deref().unwrap_or(""))).collect()
    }

    fn symbol(item: &Item) -> &str {
        item.logo.as_deref().and_then(|url| url.strip_prefix(SYMBOLS)).map(|rest| rest.trim_start_matches('/').trim_end_matches(".png")).unwrap_or("")
    }

    #[test]
    fn a_forecast_becomes_the_place_now_and_a_tile_a_day() {
        let forecast = forecast();
        assert_eq!(sdk::epoch_ms(2026, 11, 10, 22, 15, 0, 60), THAT_EVENING_MS);
        let items = tiles(&forecast, &berlin(), false, &Moment::fresh(&forecast, THAT_EVENING_MS));
        assert_eq!(
            lines(&items),
            [
                ("place", "Berlin", "State of Berlin, Germany"),
                ("now", "3°C now", "Cloudy"),
                ("day:0", "5° today", "Light rain"),
                ("day:1", "5° tomorrow", "Light rain"),
                ("day:2", "6° Thursday", "Cloudy"),
                ("day:3", "8° Friday", "Light showers"),
                ("day:4", "7° Saturday", "Partly cloudy"),
                ("day:5", "6° Sunday", "Partly cloudy"),
                ("day:6", "9° Monday", "Light rain"),
            ]
        );
        assert_eq!(items.iter().map(symbol).collect::<Vec<_>>(), ["", "cloudy", "lightrain", "lightrain", "cloudy", "lightrainshowers_day", "partlycloudy_day", "partlycloudy_day", "lightrain"]);
        // In order, in the plugin's one row, and read rather than opened.
        assert!(items.windows(2).all(|pair| pair[0].weight > pair[1].weight));
        let rows: Vec<String> = <Weather as sdk::Plugin>::manifest().rows.into_iter().map(|r| r.id).collect();
        assert!(items.iter().all(|i| i.row.as_ref().is_some_and(|r| rows.contains(r)) && i.action.is_empty()));
    }

    #[test]
    fn a_fresh_forecast_starts_today_whatever_the_clock_says() {
        let forecast = forecast();
        // A TV whose clock runs a day ahead still calls the forecast's first day today.
        let items = tiles(&forecast, &berlin(), false, &Moment::fresh(&forecast, THAT_EVENING_MS + DAY_MS));
        assert_eq!(lines(&items)[2], ("day:0", "5° today", "Light rain"));
    }

    #[test]
    fn a_kept_forecast_is_named_against_the_clock() {
        let forecast = forecast();
        let shown = |after_ms: i64| {
            let items = tiles(&forecast, &berlin(), false, &Moment::later(&forecast, THAT_EVENING_MS + after_ms, after_ms));
            items.iter().skip(1).map(|i| format!("{}: {}", i.id, i.title)).collect::<Vec<_>>()
        };
        // Half an hour on, the reading still stands for now.
        assert_eq!(shown(30 * 60_000)[..2], ["now: 3°C now", "day:0: 5° today"]);
        // Two hours on it is past midnight in Berlin: the reading is old and Wednesday is today.
        assert_eq!(shown(2 * 3_600_000), ["day:0: 5° today", "day:1: 6° tomorrow", "day:2: 8° Friday", "day:3: 7° Saturday", "day:4: 6° Sunday", "day:5: 9° Monday"]);
        assert_eq!(shown(5 * DAY_MS + 2 * 3_600_000), ["day:0: 9° today"]);
        // Once every day has passed only the place is left, and the refresh puts the row away.
        assert!(shown(8 * DAY_MS).is_empty());
        assert_eq!((span(40 * 60_000), span(60 * 60_000), span(5 * 3_600_000), span(3 * DAY_MS), span(0)), ("40 minutes".into(), "1 hour".into(), "5 hours".into(), "3 days".into(), "1 minute".into()));
        // What is kept comes back as it went.
        let kept = Kept { asked: "Berlin".into(), fahrenheit: false, fetched_ms: THAT_EVENING_MS, forecast };
        let back: Kept = serde_json::from_slice(&serde_json::to_vec(&kept).unwrap()).unwrap();
        assert_eq!((back.asked.as_str(), back.fetched_ms, back.forecast.utc_offset_seconds, back.forecast.daily.time.len()), ("Berlin", THAT_EVENING_MS, 3600, 7));
    }

    #[test]
    fn the_manifest_is_sound() {
        let manifest = <Weather as sdk::Plugin>::manifest();
        manifest.validate().unwrap();
        for url in [FORECAST, GEOCODING, PLACE_BY_ID, SYMBOLS] {
            assert!(manifest.allows_host(&sdk::host_of(url).unwrap()), "{url}");
        }
    }

    #[test]
    fn every_code_has_words_that_fit_and_a_symbol_that_is_served() {
        let served: Vec<&str> = SERVED.split_whitespace().collect();
        for code in CODES {
            for lit in [true, false] {
                let (words, symbol) = sky(code as f64, lit).unwrap_or_else(|| panic!("no words for {code}"));
                assert!(served.contains(&symbol.as_str()), "{symbol} for {code}");
                // A compact tile's second line fits about this much.
                assert!(words.len() <= 18, "{words}");
            }
        }
        assert_eq!(sky(0.0, true).unwrap(), ("Sunny", "clearsky_day".to_string()));
        assert_eq!(sky(0.0, false).unwrap(), ("Clear", "clearsky_night".to_string()));
        assert!(sky(4.0, true).is_none() && sky(-1.0, true).is_none() && sky(100.0, false).is_none());
    }

    #[test]
    fn now_names_its_scale_and_follows_the_light() {
        let said = |temperature, is_day, code, fahrenheit| {
            let forecast = Forecast { utc_offset_seconds: 0, current: Current { temperature_2m: temperature, is_day, weather_code: code }, daily: Daily::default() };
            let now = tiles(&forecast, &berlin(), fahrenheit, &Moment { today: 0, now: true }).remove(1);
            (now.title.clone(), now.subtitle.clone().unwrap_or_default(), symbol(&now).to_string())
        };
        assert_eq!(said(21.4, Some(1.0), Some(0.0), false), ("21°C now".into(), "Sunny".into(), "clearsky_day".into()));
        assert_eq!(said(69.5, Some(0.0), Some(1.0), true), ("70°F now".into(), "Mostly clear".into(), "fair_night".into()));
        // Without word of the light it is taken for day, and below zero wears a true minus.
        assert_eq!(said(-12.6, None, Some(80.0), false), ("\u{2212}13°C now".into(), "Light showers".into(), "lightrainshowers_day".into()));
        // A code this plugin does not know leaves the words and the symbol out.
        assert_eq!(said(-0.4, Some(1.0), Some(42.0), false), ("0°C now".into(), String::new(), String::new()));
        assert_eq!(said(104.0, Some(1.0), None, true), ("104°F now".into(), String::new(), String::new()));
    }

    #[test]
    fn days_the_forecast_does_not_reach_are_left_out() {
        let forecast: Forecast = serde_json::from_str(
            r#"{"current":{"temperature_2m":30.0},
                "daily":{"time":["2026-02-28","2026-03-01","2026-03-02","soon","2026-03-04","2026-03-04"],
                    "weather_code":[null,95,0],
                    "temperature_2m_max":[31.2,null,28.5,27.0,26.0,25.0]}}"#,
        )
        .unwrap();
        let items = tiles(&forecast, &berlin(), false, &Moment::fresh(&forecast, 0));
        assert_eq!(lines(&items)[1..], [("now", "30°C now", ""), ("day:0", "31° today", ""), ("day:2", "29° Monday", "Sunny"), ("day:4", "26° Wednesday", "")]);
        // Without a day there is no week to show, and an error body is not a forecast.
        let bare: Forecast = serde_json::from_str(r#"{"current":{"temperature_2m":30.0}}"#).unwrap();
        assert!(!tiles(&bare, &berlin(), false, &Moment::fresh(&bare, 0)).iter().any(|i| i.id.starts_with("day:")));
        assert!(serde_json::from_str::<Forecast>(r#"{"error":true,"reason":"Latitude must be in range of -90 to 90°. Given: 999.0."}"#).is_err());
    }

    #[test]
    fn a_day_without_daylight_is_clear_not_sunny() {
        let forecast: Forecast = serde_json::from_str(
            r#"{"current":{"temperature_2m":-21.0,"is_day":0,"weather_code":0},
                "daily":{"time":["2026-12-20","2026-12-21","2026-12-22"],"weather_code":[0,1,2],
                    "temperature_2m_max":[-20.6,-22.0,-19.0],"daylight_duration":[0.0,0,null]}}"#,
        )
        .unwrap();
        let items = tiles(&forecast, &berlin(), false, &Moment::fresh(&forecast, 0));
        assert_eq!(lines(&items)[2..], [("day:0", "\u{2212}21° today", "Clear"), ("day:1", "\u{2212}22° tomorrow", "Mostly clear"), ("day:2", "\u{2212}19° Tuesday", "Partly cloudy")]);
        assert_eq!(items[2..].iter().map(symbol).collect::<Vec<_>>(), ["clearsky_night", "fair_night", "partlycloudy_day"]);
    }

    #[test]
    fn dates_fall_on_their_days() {
        let named = |date: &str| day_number(date).map(|day| day_name(2, day));
        assert_eq!(day_number("1970-01-01"), Some(0));
        assert_eq!(named("1970-01-01"), Some("Thursday"));
        assert_eq!(named("2026-11-11"), Some("Wednesday"));
        assert_eq!(named("2024-02-29"), Some("Thursday"));
        assert_eq!(named("2027-01-01"), Some("Friday"));
        for bad in ["", "2026", "2026-11", "2026-13-01", "2026-11-00", "2026-11-12T00:00", "tomorrow"] {
            assert_eq!(day_number(bad), None, "{bad}");
        }
        assert_eq!((day_name(0, 0), day_name(1, 0)), ("today", "tomorrow"));
    }

    #[test]
    fn coordinates_are_told_from_names_and_postcodes() {
        for (typed, read) in [
            ("45.76, 4.84", (45.76, 4.84)),
            ("45.76,4.84", (45.76, 4.84)),
            ("-37.81 144.96", (-37.81, 144.96)),
            ("64.15;-21.94", (64.15, -21.94)),
            ("45,76; 4,84", (45.76, 4.84)),
            ("45,76 4,84", (45.76, 4.84)),
            ("45, 4", (45.0, 4.0)),
            ("52 13.4", (52.0, 13.4)),
        ] {
            assert_eq!(coordinates(typed), Some(read), "{typed}");
        }
        for name in ["Lyon", "90210", "040 01", "10 115", "Portland, Oregon", "100 Mile House", "91.0, 4.0", "45.0, 181.0", "45.76, 4.84, 3", "nan, 4", "Paris, 75", "4,", ""] {
            assert_eq!(coordinates(name), None, "{name}");
        }
    }

    #[test]
    fn a_place_found_is_named_by_town_and_region() {
        let found: Geocoding = serde_json::from_str(
            r#"{"results":[{"id":5746545,"name":"Portland","latitude":45.52345,"longitude":-122.67621,"feature_code":"PPLA2","country_code":"US","population":652503,"country":"United States","admin1":"Oregon","admin2":"Multnomah"}],"generationtime_ms":0.6}"#,
        )
        .unwrap();
        assert_eq!(size(&found.results), 652_503.0);
        let place = found.results.into_iter().next().unwrap().into_place("Portland, Oregon");
        assert_eq!((place.town.as_deref(), place.region.as_deref(), place.name().as_str()), (Some("Portland"), Some("Oregon, United States"), "Portland, Oregon, United States"));
        assert_eq!(serde_json::from_slice::<Place>(&serde_json::to_vec(&place).unwrap()).unwrap(), place);
        assert_eq!(lines(&[place_tile(&place)]), [("place", "Portland", "Oregon, United States")]);

        let found = |name: &str, admin1: Option<&str>, country: Option<&str>, population: Option<f64>, code: Option<&str>| Found {
            id: None,
            name: name.into(),
            latitude: 1.0,
            longitude: 2.0,
            admin1: admin1.map(Into::into),
            country: country.map(Into::into),
            population,
            feature_code: code.map(Into::into),
        };
        let city_state = found("Singapore", Some("Singapore"), Some("Singapore"), Some(3_547_809.0), Some("PPLC")).into_place("singapore");
        assert_eq!((city_state.region.as_deref(), city_state.name().as_str()), (None, "Singapore"));
        assert_eq!(found("São Paulo", Some("São Paulo"), Some("Brazil"), None, None).into_place("x").region.as_deref(), Some("Brazil"));
        // Coordinates typed in have no town, so the tile shows them to the kilometre.
        let typed = Place { asked: "-37.813611, 144.963056".into(), town: None, region: None, latitude: -37.813_611, longitude: 144.963_056 };
        assert_eq!(lines(&[place_tile(&typed)]), [("place", "-37.81, 144.96", "")]);
    }

    #[test]
    fn a_name_that_is_nothing_much_in_english_is_asked_for_in_other_languages() {
        let found = |population: Option<f64>, code: &str| vec![Found { id: None, name: "Wien".into(), latitude: 0.0, longitude: 0.0, admin1: None, country: None, population, feature_code: Some(code.into()) }];
        // Vienna under its own name outweighs a hamlet in Missouri, and a country is no town.
        assert!(size(&found(None, "PPL")) < KNOWN_TOWN && size(&found(Some(421.0), "PPL")) < KNOWN_TOWN);
        assert!(size(&found(Some(1_691_468.0), "PPLC")) >= CITY);
        assert_eq!(size(&found(Some(19_020_271.0), "PCLI")), 0.0);
        assert!(size(&[]) < size(&found(None, "PPL")));
        assert_eq!(languages_for("Wien"), LATIN);
        assert_eq!(languages_for("København"), LATIN);
        assert_eq!(languages_for("Hà Nội"), LATIN);
        assert_eq!(languages_for("Москва"), ["ru", "uk", "bg"]);
        assert_eq!(languages_for("Αθήνα"), ["el"]);
        assert_eq!(languages_for("תל אביב"), ["he"]);
        assert_eq!(languages_for("القاهرة"), ["ar"]);
        assert_eq!(languages_for("กรุงเทพ"), ["th"]);
        assert_eq!(languages_for("東京"), ["zh", "ja"]);
        assert!(languages_for("서울").is_empty());
        // Found as Wien in German, the place is asked for again by its id and named in English.
        let vienna: Found = serde_json::from_str(
            r#"{"id":2761369,"name":"Vienna","latitude":48.20849,"longitude":16.37208,"feature_code":"PPLC","country_code":"AT","population":1691468,"postcodes":["1010"],"country":"Austria","admin1":"State of Vienna","admin2":"Vienna"}"#,
        )
        .unwrap();
        assert_eq!((vienna.id, vienna.into_place("Wien").name().as_str()), (Some(2_761_369), "Vienna, State of Vienna, Austria"));
        // A name nobody knows comes back with no list at all.
        assert!(serde_json::from_str::<Geocoding>(r#"{"generationtime_ms":0.11}"#).unwrap().results.is_empty());
        assert_eq!(unknown_place("Portland Maine"), "Open-Meteo couldn't find \"Portland Maine\". Check the spelling, try the name in English, or put a comma before the region, as in Portland, Maine.");
    }

    #[test]
    fn a_failure_that_may_pass_is_told_from_one_that_will_not() {
        assert_eq!(failed_with(503), Failure::Passing("Open-Meteo answered with error 503. Try again later.".into()));
        assert!(matches!(failed_with(429), Failure::Passing(_)));
        assert!(matches!(failed_with(400), Failure::Lasting(_)));
        assert_eq!(String::from(failed_with(404)), "Open-Meteo answered with error 404. Try again later.");
        // Off the TV there is no network, which is a failure that may pass.
        assert!(matches!(get_json::<Forecast>(FORECAST), Err(Failure::Passing(_))));
    }

    #[test]
    fn the_request_names_the_place_to_the_kilometre_and_the_unit() {
        let place = Place { asked: "home".into(), town: None, region: None, latitude: -37.813_611, longitude: 144.963_056 };
        let url = forecast_url(&place, false);
        assert!(url.starts_with("https://api.open-meteo.com/v1/forecast?latitude=-37.81&longitude=144.96&current="), "{url}");
        assert!(url.ends_with("&timezone=auto&forecast_days=7"), "{url}");
        assert!(forecast_url(&place, true).ends_with("&forecast_days=7&temperature_unit=fahrenheit"));
        assert_eq!(encode_query("São Paulo, SP"), "S%C3%A3o%20Paulo%2C%20SP");
    }

    #[test]
    fn the_log_says_where_and_how_it_is() {
        let forecast = forecast();
        assert_eq!(summary(&berlin(), &forecast.current, false), "3°C and cloudy in Berlin, State of Berlin, Germany");
        let typed = Place { asked: "52.52, 13.41".into(), town: None, region: None, latitude: 52.52, longitude: 13.41 };
        assert_eq!(summary(&typed, &Current { temperature_2m: 38.0, is_day: None, weather_code: None }, true), "38°F in 52.52, 13.41");
    }
}
