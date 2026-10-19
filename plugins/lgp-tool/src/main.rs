//! `lgp-tool`: the plugin developer's command line.
//!
//! ```text
//! lgp-tool keygen <key-file>                       write a new Ed25519 signing key (hex seed)
//! lgp-tool bundle <plugin.wasm> <key-file> <out.lgp> [--manifest manifest.json]
//! lgp-tool inspect <bundle.lgp>                    print the manifest and verify the signature
//! lgp-tool run <plugin.wasm|bundle.lgp> [--set key=value]... [--secret key=value]... [--fixtures dir] [--search query] [--resolve action]
//! ```
//! `run` executes `refresh` (or `search`) with a host that keeps key-values in memory, reads
//! settings from the flags, and fetches live unless `--fixtures` points at a directory of
//! `<sha256(url)>.body` files.

use ed25519_dalek::{Signer, SigningKey};
use lounge_plugin_api::{HttpRequest, HttpResponse, Manifest, RefreshContext};
use lounge_plugins::bundle::Bundle;
use lounge_plugins::host::{Host, Limits, Services};
use lounge_plugins::http::Fetcher;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("keygen") => keygen(&args[1..]),
        Some("bundle") => bundle(&args[1..]),
        Some("inspect") => inspect(&args[1..]),
        Some("run") => run(&args[1..]),
        _ => Err(USAGE.to_string()),
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

const USAGE: &str = "usage: lgp-tool keygen <key> | bundle <wasm> <key> <out.lgp> [--manifest m.json] | inspect <lgp> | run <wasm|lgp> [--set k=v] [--secret k=v] [--fixtures dir] [--search q] [--resolve action]";

fn keygen(args: &[String]) -> Result<(), String> {
    let out = args.first().ok_or(USAGE)?;
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| e.to_string())?;
    let key = SigningKey::from_bytes(&seed);
    std::fs::write(out, hex::encode(seed)).map_err(|e| e.to_string())?;
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
    let services = Arc::new(RunServices {
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
    let plugin = host.load(&manifest.id.clone(), manifest, &wasm).map_err(|e| e.to_string())?;
    let started = std::time::Instant::now();
    if let Some(action) = flag(args, "--resolve") {
        let resolved = host.resolve(&plugin, &action).map_err(|e| e.to_string())?;
        for line in services.logs.lock().iter() {
            println!("  log: {line}");
        }
        println!("resolved in {:.0} ms: {resolved:?}", started.elapsed().as_secs_f64() * 1000.0);
        return Ok(());
    }
    let items = match flag(args, "--search") {
        Some(q) => host.search(&plugin, &q, 20),
        None => host.refresh(
            &plugin,
            &RefreshContext {
                now_ms: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0),
                utc_offset_minutes: 0,
                tier: "a".into(),
                since_ms: None,
                window_hours: 6,
            },
        ),
    }
    .map_err(|e| e.to_string())?;
    let elapsed = started.elapsed();
    for line in services.logs.lock().iter() {
        println!("  log: {line}");
    }
    println!("{} items in {:.0} ms", items.len(), elapsed.as_secs_f64() * 1000.0);
    for it in items.iter().take(40) {
        println!(
            "  [{:?}] {}{}{}{}",
            it.kind,
            it.title,
            it.subtitle.as_ref().map(|s| format!(" — {s}")).unwrap_or_default(),
            it.progress.map(|p| format!(" ({:.0}%)", p * 100.0)).unwrap_or_default(),
            it.channel_number.as_ref().map(|n| format!(" ch {n}")).unwrap_or_default(),
        );
    }
    if items.len() > 40 {
        println!("  … and {} more", items.len() - 40);
    }
    Ok(())
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn pairs(args: &[String], name: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for (i, a) in args.iter().enumerate() {
        if a == name {
            if let Some((k, v)) = args.get(i + 1).and_then(|kv| kv.split_once('=')) {
                out.insert(k.to_string(), v.to_string());
            }
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
            use sha2::Digest;
            let name = hex::encode(sha2::Sha256::digest(request.url.as_bytes()));
            let body = std::fs::read_to_string(dir.join(format!("{name}.body")))
                .map_err(|_| format!("no fixture for {} ({name}.body)", request.url))?;
            return Ok(HttpResponse { status: 200, headers: Default::default(), body, body_base64: None });
        }
        Fetcher::new().fetch(request, &|_| true).map_err(|e| e.to_string())
    }
}
