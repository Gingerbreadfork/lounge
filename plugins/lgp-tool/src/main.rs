//! `lgp-tool`: the plugin developer's command line.
//!
//! ```text
//! lgp-tool keygen <key-file>                       write a new Ed25519 signing key (hex seed, mode 0600, never over a file)
//! lgp-tool bundle <plugin.wasm> <key-file> <out.lgp> [--manifest manifest.json]
//! lgp-tool inspect <bundle.lgp>                    print the manifest and verify the signature
//! lgp-tool run <plugin.wasm|bundle.lgp> [--set key=value]... [--secret key=value]... [--fixtures dir] [--state file] [--search query] [--resolve action] [--json]
//! ```
//! `run` executes `refresh` (or `search`) with a host that keeps key-values in memory, or in
//! the `--state` file between runs as the TV keeps them between refreshes, reads settings from
//! the flags, and fetches live unless `--fixtures` points at a directory of fixtures; [`FIXTURES`]
//! describes them. It prints the log, the fuel used and the items, or with `--json` every item
//! as JSON and the log on stderr.

use ed25519_dalek::{Signer, SigningKey};
use lounge_plugin_api::{HttpRequest, HttpResponse, Manifest, RefreshContext};
use lounge_plugins::bundle::Bundle;
use lounge_plugins::host::{kept, kept_results, Host, Limits, Services, SEARCH_LIMIT, WINDOW_HOURS};
use lounge_plugins::http::{host_permitted, reach, Fetcher};
use parking_lot::Mutex;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("keygen") => keygen(&args[1..]),
        Some("bundle") => bundle(&args[1..]),
        Some("inspect") => inspect(&args[1..]),
        Some("run") => run(&args[1..]),
        _ => Err(format!("{USAGE}\n\n{FIXTURES}")),
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

const USAGE: &str = "usage: lgp-tool keygen <key> | bundle <wasm> <key> <out.lgp> [--manifest m.json] | inspect <lgp> | run <wasm|lgp> [--set k=v] [--secret k=v] [--fixtures dir] [--state file] [--search q] [--resolve action] [--json]";

const FIXTURES: &str = "fixtures: --fixtures dir answers each fetch from <sha256 of the url, hex>.body in dir.
The body may be binary, such as a gzip file: as on the TV, it reaches the plugin as text when it is UTF-8 and
as body_base64 otherwise, and a body over the TV's 8 MB limit fails the fetch. An optional <sha256>.head
beside it gives the status on its first line (\"404\" or \"HTTP/1.1 404 Not Found\") and headers on the
lines after it (\"WWW-Authenticate: Digest realm=...\"). Without one the answer is 200 with no headers.";

fn keygen(args: &[String]) -> Result<(), String> {
    let out = args.first().ok_or(USAGE)?;
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| e.to_string())?;
    let key = SigningKey::from_bytes(&seed);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(out).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => format!("{out} already exists; keygen never replaces a signing key"),
        _ => format!("{out}: {e}"),
    })?;
    std::io::Write::write_all(&mut file, hex::encode(seed).as_bytes()).map_err(|e| format!("{out}: {e}"))?;
    println!("wrote {out}\npublisher key: {}", hex::encode(key.verifying_key().as_bytes()));
    Ok(())
}

fn load_key(path: &str) -> Result<SigningKey, String> {
    let hex_seed = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let seed: [u8; 32] = hex::decode(hex_seed.trim())
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|_| "key must be 32 bytes".to_string())?;
    Ok(SigningKey::from_bytes(&seed))
}

fn bundle(args: &[String]) -> Result<(), String> {
    let wasm_path = args.first().ok_or(USAGE)?;
    let key_path = args.get(1).ok_or(USAGE)?;
    let out = args.get(2).ok_or(USAGE)?;
    let manifest_path = flag(args, "--manifest");
    let wasm = std::fs::read(wasm_path).map_err(|e| format!("{wasm_path}: {e}"))?;

    // The manifest comes from the module itself unless one is given; either way it must match.
    let host = Host::new(Arc::new(RunServices::default()), Limits::default());
    let reported = host
        .manifest_of(&host.load("probe", Manifest::default(), &wasm).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let manifest_json = match manifest_path {
        Some(p) => {
            let bytes = std::fs::read(&p).map_err(|e| format!("{p}: {e}"))?;
            let given: Manifest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if given != reported {
                return Err("manifest.json differs from what the module reports".into());
            }
            bytes
        }
        None => serde_json::to_vec_pretty(&reported).map_err(|e| e.to_string())?,
    };
    reported.validate()?;
    let key = load_key(key_path)?;
    let sig = key.sign(&Bundle::signed_bytes(&manifest_json, &wasm));
    Bundle::write(Path::new(out), &manifest_json, &wasm, key.verifying_key().as_bytes(), &sig.to_bytes()).map_err(|e| e.to_string())?;
    println!("{out}: {} v{} ({} bytes of wasm), publisher {}", reported.id, reported.version, wasm.len(), hex::encode(key.verifying_key().as_bytes()));
    Ok(())
}

fn inspect(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or(USAGE)?;
    let b = Bundle::read(Path::new(path)).map_err(|e| e.to_string())?;
    b.verify().map_err(|e| e.to_string())?;
    println!("{}", serde_json::to_string_pretty(&b.manifest).map_err(|e| e.to_string())?);
    println!("publisher {}\nsignature ok, {} bytes of wasm", b.publisher_hex(), b.wasm.len());
    Ok(())
}

fn run(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or(USAGE)?;
    let (manifest, wasm) = if path.ends_with(".lgp") {
        let b = Bundle::read(Path::new(path)).map_err(|e| e.to_string())?;
        b.verify().map_err(|e| e.to_string())?;
        (Some(b.manifest), b.wasm)
    } else {
        (None, std::fs::read(path).map_err(|e| format!("{path}: {e}"))?)
    };
    let state = flag(args, "--state").map(PathBuf::from);
    let services = Arc::new(RunServices {
        kv: Mutex::new(state.as_deref().map(load_state).transpose()?.unwrap_or_default()),
        settings: Mutex::new(pairs(args, "--set")),
        secrets: Mutex::new(pairs(args, "--secret")),
        fixtures: flag(args, "--fixtures").map(PathBuf::from),
        ..Default::default()
    });
    let host = Host::new(services.clone(), Limits::default());
    let manifest = match manifest {
        Some(m) => m,
        None => host.manifest_of(&host.load("probe", Manifest::default(), &wasm).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?,
    };
    let _ = services.manifest.set(manifest.clone());
    let plugin = host.load(&manifest.id.clone(), manifest, &wasm).map_err(|e| e.to_string())?;
    let fuel_used = Arc::new(Mutex::new(None));
    let _ = tracing::subscriber::set_global_default(FuelMeter(fuel_used.clone()));
    let started = std::time::Instant::now();
    if let Some(action) = flag(args, "--resolve") {
        let resolved = host.resolve(&plugin, &action).inspect_err(|_| print_logs(&services)).map_err(|e| e.to_string())?;
        print_logs(&services);
        println!("resolved in {:.0} ms: {resolved:?}", started.elapsed().as_secs_f64() * 1000.0);
        return Ok(());
    }
    // What the TV would keep of the answer, with the notes it would put in the plugin's log.
    let items = match flag(args, "--search") {
        Some(q) => host.search(&plugin, &q, SEARCH_LIMIT).map(kept_results),
        None => host
            .refresh_counted(
                &plugin,
                &RefreshContext {
                    now_ms: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0),
                    utc_offset_minutes: 0,
                    tier: "a".into(),
                    since_ms: None,
                    window_hours: WINDOW_HOURS,
                },
            )
            .map(|(items, sent)| {
                let (items, notes) = kept(items, sent);
                services.logs.lock().extend(notes.into_iter().map(|n| format!("tv: {n}")));
                items
            }),
    }
    .inspect_err(|_| print_logs(&services));
    if let Some(file) = &state {
        save_state(file, &services.kv.lock())?;
    }
    let items = items.map_err(|e| e.to_string())?;
    let elapsed = started.elapsed();
    if args.iter().any(|a| a == "--json") {
        for line in services.logs.lock().iter() {
            eprintln!("log: {line}");
        }
        println!("{}", serde_json::to_string_pretty(&items).map_err(|e| e.to_string())?);
        return Ok(());
    }
    print_logs(&services);
    let fuel = fuel_used.lock().map(|f| format!(", {} of {} fuel", amount(f), amount(Limits::default().fuel))).unwrap_or_default();
    println!("{} items in {:.0} ms{fuel}", items.len(), elapsed.as_secs_f64() * 1000.0);
    for it in items.iter().take(40) {
        println!(
            "  [{:?}] {}{}{}{}",
            it.kind,
            it.title,
            it.subtitle.as_ref().map(|s| format!(" — {s}")).unwrap_or_default(),
            it.progress.map(|p| format!(" ({:.0}%)", p * 100.0)).unwrap_or_default(),
            it.channel_number.as_ref().map(|n| format!(" ch {n}")).unwrap_or_default(),
        );
        if let Some(shader) = &it.shader {
            let names: Vec<&str> = shader.uniforms.keys().map(String::as_str).collect();
            println!("      shader: {} bytes, uniforms set: {}", shader.source.len(), if names.is_empty() { "none".into() } else { names.join(", ") });
        }
    }
    if items.len() > 40 {
        println!("  … and {} more", items.len() - 40);
    }
    Ok(())
}

fn amount(n: u64) -> String {
    if n >= 1_000_000_000 { format!("{:.2} billion", n as f64 / 1e9) } else { format!("{:.1} million", n as f64 / 1e6) }
}

fn print_logs(services: &RunServices) {
    for line in services.logs.lock().iter() {
        println!("  log: {line}");
    }
}

/// Catches the host's debug line saying how much fuel a call used.
struct FuelMeter(Arc<Mutex<Option<u64>>>);

impl tracing::Subscriber for FuelMeter {
    fn enabled(&self, metadata: &tracing::Metadata) -> bool {
        metadata.target().starts_with("lounge_plugins")
    }
    fn new_span(&self, _: &tracing::span::Attributes) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event) {
        struct Message(String);
        impl tracing::field::Visit for Message {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.0 = format!("{value:?}");
                }
            }
        }
        let mut message = Message(String::new());
        event.record(&mut message);
        if let Some(n) = message.0.strip_suffix(" fuel").and_then(|m| m.rsplit(' ').next()).and_then(|n| n.parse().ok()) {
            *self.0.lock() = Some(n);
        }
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

/// The key-value store as a JSON object: text values as strings, others as `{"base64": …}`.
fn load_state(file: &Path) -> Result<HashMap<String, Vec<u8>>, String> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(e) => return Err(format!("{}: {e}", file.display())),
    };
    let map: HashMap<String, serde_json::Value> = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    map.into_iter()
        .map(|(k, v)| {
            let bytes = match v {
                serde_json::Value::String(s) => s.into_bytes(),
                serde_json::Value::Object(o) => match o.get("base64").and_then(|b| b.as_str()) {
                    Some(b) => lounge_plugin_api::base64_decode(b),
                    None => return Err(format!("{}: {k}: expected a string or {{\"base64\": …}}", file.display())),
                },
                _ => return Err(format!("{}: {k}: expected a string or {{\"base64\": …}}", file.display())),
            };
            Ok((k, bytes))
        })
        .collect()
}

fn save_state(file: &Path, kv: &HashMap<String, Vec<u8>>) -> Result<(), String> {
    let mut map = serde_json::Map::new();
    let mut keys: Vec<&String> = kv.keys().collect();
    keys.sort();
    for k in keys {
        let v = &kv[k];
        map.insert(
            k.clone(),
            match std::str::from_utf8(v) {
                Ok(s) => serde_json::Value::String(s.to_string()),
                Err(_) => serde_json::json!({"base64": lounge_plugin_api::base64_encode(v)}),
            },
        );
    }
    std::fs::write(file, serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", file.display()))
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn pairs(args: &[String], name: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for (i, a) in args.iter().enumerate() {
        if a == name
            && let Some((k, v)) = args.get(i + 1).and_then(|kv| kv.split_once('='))
        {
            out.insert(k.to_string(), v.to_string());
        }
    }
    out
}

#[derive(Default)]
struct RunServices {
    kv: Mutex<HashMap<String, Vec<u8>>>,
    settings: Mutex<HashMap<String, String>>,
    secrets: Mutex<HashMap<String, String>>,
    logs: Mutex<Vec<String>>,
    fixtures: Option<PathBuf>,
    /// The plugin's manifest once it is known, for the TV's rules on where fetches may go.
    manifest: std::sync::OnceLock<Manifest>,
}

impl Services for RunServices {
    fn kv_get(&self, _p: &str, key: &str) -> Option<Vec<u8>> {
        self.kv.lock().get(key).cloned()
    }
    fn kv_set(&self, _p: &str, key: &str, value: &[u8]) {
        self.kv.lock().insert(key.to_string(), value.to_vec());
    }
    fn kv_delete(&self, _p: &str, key: &str) {
        self.kv.lock().remove(key);
    }
    fn secret(&self, _p: &str, key: &str) -> Option<String> {
        self.secrets.lock().get(key).cloned()
    }
    fn setting(&self, _p: &str, key: &str) -> Option<String> {
        self.settings.lock().get(key).cloned()
    }
    fn log(&self, _p: &str, level: i32, message: &str) {
        let tag = ["debug", "info", "warn", "error"].get(level as usize).unwrap_or(&"log");
        self.logs.lock().push(format!("{tag}: {message}"));
    }
    fn fetch(&self, _p: &str, request: &HttpRequest) -> Result<HttpResponse, String> {
        if let Some(dir) = &self.fixtures {
            return fixture(dir, &request.url);
        }
        let manifest = self.manifest.get().ok_or("nothing may be fetched while the manifest is read")?;
        // The TV's own rules: the allow list on every hop, and loopback and the home network
        // only where the TV would let the plugin reach them.
        let setting = |key: &str| self.settings.lock().get(key).cloned();
        Fetcher::guarded(reach(manifest, &setting)).fetch(request, &|host| host_permitted(manifest, host, &setting)).map_err(|e| e.to_string())
    }
}

/// The recorded answer for `url`, shaped the way the TV's host shapes a live one.
fn fixture(dir: &Path, url: &str) -> Result<HttpResponse, String> {
    use sha2::Digest;
    let name = hex::encode(sha2::Sha256::digest(url.as_bytes()));
    let bytes = std::fs::read(dir.join(format!("{name}.body"))).map_err(|_| format!("no fixture for {url} ({name}.body)"))?;
    let limit = Fetcher::new().max_body;
    if bytes.len() > limit {
        return Err(format!("fetch: the response body is larger than request limit: {limit}"));
    }
    let (status, headers) = match std::fs::read_to_string(dir.join(format!("{name}.head"))) {
        Ok(head) => parse_head(&head).map_err(|e| format!("{name}.head: {e}"))?,
        Err(_) => (200, Default::default()),
    };
    Ok(match String::from_utf8(bytes) {
        Ok(body) => HttpResponse { status, headers, body, body_base64: None },
        Err(e) => HttpResponse { status, headers, body: String::new(), body_base64: Some(lounge_plugin_api::base64_encode(e.as_bytes())) },
    })
}

/// A status line ("404" or "HTTP/1.1 404 Not Found") and then "Name: value" headers, with
/// names lowercased as the host does.
fn parse_head(head: &str) -> Result<(u16, BTreeMap<String, String>), String> {
    let mut lines = head.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next().ok_or("empty")?;
    let code = if first.starts_with("HTTP/") { first.split_whitespace().nth(1).unwrap_or("") } else { first };
    let status = code.parse::<u16>().map_err(|_| format!("{first:?} is not a status"))?;
    let mut headers = BTreeMap::new();
    for line in lines {
        let (k, v) = line.split_once(':').ok_or_else(|| format!("{line:?} is not a header"))?;
        headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
    }
    Ok((status, headers))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lgp-tool-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn file_for(dir: &Path, url: &str, ext: &str) -> PathBuf {
        use sha2::Digest;
        dir.join(format!("{}.{ext}", hex::encode(sha2::Sha256::digest(url.as_bytes()))))
    }

    #[test]
    fn fixtures_carry_binary_bodies_statuses_and_headers() {
        let d = dir("fixtures");
        std::fs::write(file_for(&d, "http://a/text", "body"), "hello").unwrap();
        std::fs::write(file_for(&d, "http://a/gz", "body"), [0x1f, 0x8b, 0x08, 0xff]).unwrap();
        std::fs::write(file_for(&d, "http://a/locked", "body"), "").unwrap();
        std::fs::write(file_for(&d, "http://a/locked", "head"), "HTTP/1.1 401 Unauthorized\nWWW-Authenticate: Digest realm=\"tvheadend\", nonce=\"n\"\n").unwrap();
        let text = fixture(&d, "http://a/text").unwrap();
        assert_eq!((text.status, text.body.as_str(), text.body_base64.is_none()), (200, "hello", true));
        let gz = fixture(&d, "http://a/gz").unwrap();
        assert_eq!(gz.bytes(), [0x1f, 0x8b, 0x08, 0xff]);
        assert!(gz.body.is_empty());
        let locked = fixture(&d, "http://a/locked").unwrap();
        assert_eq!(locked.status, 401);
        assert_eq!(locked.headers["www-authenticate"], "Digest realm=\"tvheadend\", nonce=\"n\"");
        assert!(fixture(&d, "http://a/missing").unwrap_err().contains("no fixture"));
        std::fs::write(file_for(&d, "http://a/huge", "body"), vec![b'x'; Fetcher::new().max_body + 1]).unwrap();
        assert!(fixture(&d, "http://a/huge").unwrap_err().contains("larger than"));
        assert_eq!(parse_head("404").unwrap().0, 404);
        assert!(parse_head("teapot").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn keygen_never_overwrites_and_keeps_the_seed_private() {
        let d = dir("keygen");
        let path = d.join("publisher.key").to_string_lossy().into_owned();
        keygen(std::slice::from_ref(&path)).unwrap();
        let first = std::fs::read(&path).unwrap();
        assert!(keygen(std::slice::from_ref(&path)).unwrap_err().contains("already exists"));
        assert_eq!(std::fs::read(&path).unwrap(), first);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(load_key(&path).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }
}
