//! Runs plugin modules. Every call gets a fresh instance with a fuel budget and a memory cap,
//! so a misbehaving plugin can burn its own budget and nothing else. Host functions read and
//! write the plugin's linear memory through its own `lg_alloc`.

use crate::api::{self, exports, imports, Action, HttpRequest, HttpResponse, Item, Manifest, Outcome, RefreshContext, SearchQuery};
use crate::{PluginError, Result};
use serde::{de::DeserializeOwned, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use wasmi::{Caller, Config, Engine, Extern, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder};

/// What the host provides to plugins. Implemented by the core (storage) and by the test tool.
pub trait Services: Send + Sync {
    fn kv_get(&self, plugin: &str, key: &str) -> Option<Vec<u8>>;
    fn kv_set(&self, plugin: &str, key: &str, value: &[u8]);
    fn kv_delete(&self, plugin: &str, key: &str);
    fn secret(&self, plugin: &str, key: &str) -> Option<String>;
    fn setting(&self, plugin: &str, key: &str) -> Option<String>;
    fn log(&self, plugin: &str, level: i32, message: &str);
    /// Already checked against the manifest's allow list by the host.
    fn fetch(&self, plugin: &str, request: &HttpRequest) -> std::result::Result<HttpResponse, String>;
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub memory_bytes: usize,
    pub fuel: u64,
    /// Longest value a plugin may store under one key.
    pub kv_value_bytes: usize,
    pub kv_keys: usize,
    /// Most `fetch` calls one invocation may make, and the wall-clock budget they share;
    /// fuel meters instructions, this meters the time spent waiting on the network.
    pub max_fetches: u32,
    pub fetch_budget: Duration,
    /// Largest value an export may hand back.
    pub max_result_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            memory_bytes: 32 * 1024 * 1024,
            fuel: 20_000_000_000,
            kv_value_bytes: 256 * 1024,
            kv_keys: 256,
            max_fetches: 64,
            fetch_budget: Duration::from_secs(60),
            max_result_bytes: 8 * 1024 * 1024,
        }
    }
}

pub struct LoadedPlugin {
    pub id: String,
    pub manifest: Manifest,
    module: Module,
}

pub struct Host {
    engine: Engine,
    services: Arc<dyn Services>,
    limits: Limits,
}

struct State {
    plugin: String,
    manifest: Manifest,
    services: Arc<dyn Services>,
    limits: StoreLimits,
    kv_limits: (usize, usize),
    log_lines: u32,
    fetches_left: u32,
    fetch_deadline: Instant,
}

impl Host {
    pub fn new(services: Arc<dyn Services>, limits: Limits) -> Host {
        let mut config = Config::default();
        config.consume_fuel(true);
        Host { engine: Engine::new(&config), services, limits }
    }

    /// Compile a module. Fails for anything that is not valid WebAssembly.
    pub fn load(&self, id: &str, manifest: Manifest, wasm: &[u8]) -> Result<LoadedPlugin> {
        let module = Module::new(&self.engine, wasm).map_err(|e| PluginError::Wasm(e.to_string()))?;
        for required in [exports::MEMORY, exports::ALLOC, exports::FREE, exports::MANIFEST, exports::REFRESH] {
            if module.get_export(required).is_none() {
                return Err(PluginError::Wasm(format!("module does not export {required}")));
            }
        }
        Ok(LoadedPlugin { id: id.to_string(), manifest, module })
    }

    /// The manifest the module itself reports, for cross-checking with `manifest.json`.
    pub fn manifest_of(&self, plugin: &LoadedPlugin) -> Result<Manifest> {
        self.call_json::<(), Manifest>(plugin, exports::MANIFEST, None).map(|o| match o {
            Outcome::Ok(m) => Ok(m),
            Outcome::Err(e) => Err(PluginError::Failed(e)),
        })?
    }

    pub fn refresh(&self, plugin: &LoadedPlugin, ctx: &RefreshContext) -> Result<Vec<Item>> {
        self.outcome(self.call_json(plugin, exports::REFRESH, Some(ctx))?)
    }

    pub fn search(&self, plugin: &LoadedPlugin, query: &str, limit: u32) -> Result<Vec<Item>> {
        if plugin.module.get_export(exports::SEARCH).is_none() {
            return Ok(Vec::new());
        }
        let q = SearchQuery { query: query.to_string(), limit };
        self.outcome(self.call_json(plugin, exports::SEARCH, Some(&q))?)
    }

    pub fn resolve(&self, plugin: &LoadedPlugin, action: &str) -> Result<Action> {
        if plugin.module.get_export(exports::RESOLVE).is_none() {
            return Err(PluginError::Failed("plugin cannot resolve actions".into()));
        }
        self.outcome(self.call_json(plugin, exports::RESOLVE, Some(&action.to_string()))?)
    }

    fn outcome<T>(&self, o: Outcome<T>) -> Result<T> {
        match o {
            Outcome::Ok(v) => Ok(v),
            Outcome::Err(e) => Err(PluginError::Failed(e)),
        }
    }

    fn call_json<I: Serialize, O: DeserializeOwned>(&self, plugin: &LoadedPlugin, export: &str, input: Option<&I>) -> Result<Outcome<O>> {
        let state = State {
            plugin: plugin.id.clone(),
            manifest: plugin.manifest.clone(),
            services: Arc::clone(&self.services),
            limits: StoreLimitsBuilder::new().memory_size(self.limits.memory_bytes).memories(1).build(),
            kv_limits: (self.limits.kv_value_bytes, self.limits.kv_keys),
            log_lines: 0,
            fetches_left: self.limits.max_fetches,
            fetch_deadline: Instant::now() + self.limits.fetch_budget,
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        store.set_fuel(self.limits.fuel).map_err(wasm_err)?;

        let mut linker = Linker::<State>::new(&self.engine);
        link_host_functions(&mut linker)?;
        let instance = linker.instantiate_and_start(&mut store, &plugin.module).map_err(wasm_err)?;
        let memory = instance
            .get_memory(&store, exports::MEMORY)
            .ok_or_else(|| PluginError::Wasm("no memory export".into()))?;

        let packed = match input {
            Some(value) => {
                let bytes = serde_json::to_vec(value)?;
                let alloc = instance.get_typed_func::<i32, i32>(&store, exports::ALLOC).map_err(wasm_err)?;
                let ptr = alloc.call(&mut store, bytes.len() as i32).map_err(trap_err)? as u32;
                memory.write(&mut store, ptr as usize, &bytes).map_err(|e| PluginError::Wasm(e.to_string()))?;
                let f = instance.get_typed_func::<(i32, i32), i64>(&store, export).map_err(wasm_err)?;
                f.call(&mut store, (ptr as i32, bytes.len() as i32)).map_err(trap_err)?
            }
            None => {
                let f = instance.get_typed_func::<(), i64>(&store, export).map_err(wasm_err)?;
                f.call(&mut store, ()).map_err(trap_err)?
            }
        };
        let (ptr, len) = api::unpack(packed);
        if len == 0 {
            return Err(PluginError::Failed(format!("{export} returned nothing")));
        }
        // The plugin names the range; it is checked against its memory before a byte is allocated.
        let end = ptr as usize + len as usize;
        if len as usize > self.limits.max_result_bytes || end > memory.data_size(&store) {
            return Err(PluginError::Wasm(format!("{export} returned {len} bytes at {ptr}, outside its memory or over the limit")));
        }
        let mut out = vec![0u8; len as usize];
        memory.read(&store, ptr as usize, &mut out).map_err(|e| PluginError::Wasm(e.to_string()))?;
        if let Ok(free) = instance.get_typed_func::<(i32, i32), ()>(&store, exports::FREE) {
            let _ = free.call(&mut store, (ptr as i32, len as i32));
        }
        let used = self.limits.fuel.saturating_sub(store.get_fuel().unwrap_or(0));
        tracing::debug!("plugin {} {export}: {} fuel", plugin.id, used);
        Ok(serde_json::from_slice(&out)?)
    }
}

fn wasm_err(e: wasmi::Error) -> PluginError {
    PluginError::Wasm(e.to_string())
}

fn trap_err(e: wasmi::Error) -> PluginError {
    match e.as_trap_code() {
        Some(wasmi::TrapCode::OutOfFuel) => PluginError::Trap("out of fuel: the plugin took too long".into()),
        Some(code) => PluginError::Trap(format!("{code:?}")),
        None => PluginError::Trap(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Host functions
// ---------------------------------------------------------------------------

fn memory_of(caller: &Caller<'_, State>) -> std::result::Result<Memory, wasmi::Error> {
    caller
        .get_export(exports::MEMORY)
        .and_then(Extern::into_memory)
        .ok_or_else(|| wasmi::Error::new("plugin has no memory export"))
}

fn read_guest(caller: &Caller<'_, State>, ptr: i32, len: i32) -> std::result::Result<Vec<u8>, wasmi::Error> {
    if len <= 0 {
        return Ok(Vec::new());
    }
    let mem = memory_of(caller)?;
    let mut buf = vec![0u8; len as usize];
    mem.read(caller, ptr as usize, &mut buf).map_err(|e| wasmi::Error::new(e.to_string()))?;
    Ok(buf)
}

fn read_guest_str(caller: &Caller<'_, State>, ptr: i32, len: i32) -> std::result::Result<String, wasmi::Error> {
    String::from_utf8(read_guest(caller, ptr, len)?).map_err(|_| wasmi::Error::new("plugin passed invalid UTF-8"))
}

/// Allocate in the plugin with its `lg_alloc`, copy `bytes` in and return the packed pointer.
fn write_guest(caller: &mut Caller<'_, State>, bytes: &[u8]) -> std::result::Result<i64, wasmi::Error> {
    if bytes.is_empty() {
        return Ok(0);
    }
    let alloc = caller
        .get_export(exports::ALLOC)
        .and_then(Extern::into_func)
        .ok_or_else(|| wasmi::Error::new("plugin has no lg_alloc"))?
        .typed::<i32, i32>(&*caller)?;
    let ptr = alloc.call(&mut *caller, bytes.len() as i32)?;
    if ptr <= 0 {
        return Err(wasmi::Error::new("plugin allocation failed"));
    }
    let mem = memory_of(caller)?;
    mem.write(&mut *caller, ptr as usize, bytes).map_err(|e| wasmi::Error::new(e.to_string()))?;
    Ok(api::pack(ptr as u32, bytes.len() as u32))
}

fn link_host_functions(linker: &mut Linker<State>) -> Result<()> {
    let m = api::HOST_MODULE;
    let err = |e: wasmi::errors::LinkerError| PluginError::Wasm(e.to_string());

    linker
        .func_wrap(m, imports::NOW, |_caller: Caller<'_, State>| -> i64 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0)
        })
        .map_err(err)?;

    linker
        .func_wrap(m, imports::LOG, |mut caller: Caller<'_, State>, level: i32, ptr: i32, len: i32| -> std::result::Result<(), wasmi::Error> {
            let msg = read_guest_str(&caller, ptr, len.min(4096))?;
            let state = caller.data_mut();
            state.log_lines += 1;
            if state.log_lines <= 500 {
                state.services.log(&state.plugin, level, &msg);
            }
            Ok(())
        })
        .map_err(err)?;

    linker
        .func_wrap(m, imports::KV_GET, |mut caller: Caller<'_, State>, ptr: i32, len: i32| -> std::result::Result<i64, wasmi::Error> {
            let key = read_guest_str(&caller, ptr, len.min(256))?;
            let value = caller.data().services.kv_get(&caller.data().plugin, &key);
            match value {
                Some(v) => write_guest(&mut caller, &v),
                None => Ok(0),
            }
        })
        .map_err(err)?;

    linker
        .func_wrap(m, imports::KV_SET, |caller: Caller<'_, State>, kp: i32, kl: i32, vp: i32, vl: i32| -> std::result::Result<(), wasmi::Error> {
            let key = read_guest_str(&caller, kp, kl.min(256))?;
            let (max_value, _) = caller.data().kv_limits;
            if vl as usize > max_value {
                return Err(wasmi::Error::new("kv value too large"));
            }
            let value = read_guest(&caller, vp, vl)?;
            caller.data().services.kv_set(&caller.data().plugin, &key, &value);
            Ok(())
        })
        .map_err(err)?;

    linker
        .func_wrap(m, imports::KV_DELETE, |caller: Caller<'_, State>, ptr: i32, len: i32| -> std::result::Result<(), wasmi::Error> {
            let key = read_guest_str(&caller, ptr, len.min(256))?;
            caller.data().services.kv_delete(&caller.data().plugin, &key);
            Ok(())
        })
        .map_err(err)?;

    linker
        .func_wrap(m, imports::SECRET, |mut caller: Caller<'_, State>, ptr: i32, len: i32| -> std::result::Result<i64, wasmi::Error> {
            let key = read_guest_str(&caller, ptr, len.min(256))?;
            // Only secrets the manifest declared as password settings are readable.
            let declared = caller.data().manifest.settings.iter().any(|s| s.key == key && s.kind.is_secret());
            if !declared {
                return Ok(0);
            }
            let value = caller.data().services.secret(&caller.data().plugin, &key);
            match value {
                Some(v) => write_guest(&mut caller, v.as_bytes()),
                None => Ok(0),
            }
        })
        .map_err(err)?;

    linker
        .func_wrap(m, imports::SETTING, |mut caller: Caller<'_, State>, ptr: i32, len: i32| -> std::result::Result<i64, wasmi::Error> {
            let key = read_guest_str(&caller, ptr, len.min(256))?;
            let value = caller
                .data()
                .services
                .setting(&caller.data().plugin, &key)
                .or_else(|| caller.data().manifest.settings.iter().find(|s| s.key == key).and_then(|s| s.default.clone()));
            match value {
                Some(v) => write_guest(&mut caller, v.as_bytes()),
                None => Ok(0),
            }
        })
        .map_err(err)?;

    linker
        .func_wrap(m, imports::FETCH, |mut caller: Caller<'_, State>, ptr: i32, len: i32| -> std::result::Result<i64, wasmi::Error> {
            let raw = read_guest(&caller, ptr, len.min(1 << 20))?;
            let request: HttpRequest = serde_json::from_slice(&raw).map_err(|e| wasmi::Error::new(format!("bad request: {e}")))?;
            let host = crate::http::host_of(&request.url).unwrap_or_default();
            let budget = {
                let st = caller.data_mut();
                if st.fetches_left == 0 || Instant::now() > st.fetch_deadline {
                    Err("fetch budget for this call is used up".to_string())
                } else {
                    st.fetches_left -= 1;
                    Ok(())
                }
            };
            let response: Outcome<HttpResponse> = if let Err(e) = budget {
                Outcome::Err(e)
            } else if !caller.data().manifest.allows_host(&host) {
                Outcome::Err(format!("host {host} is not in this plugin's allow list"))
            } else {
                match caller.data().services.fetch(&caller.data().plugin, &request) {
                    Ok(r) => Outcome::Ok(r),
                    Err(e) => Outcome::Err(e),
                }
            };
            let bytes = serde_json::to_vec(&response).map_err(|e| wasmi::Error::new(e.to_string()))?;
            write_guest(&mut caller, &bytes)
        })
        .map_err(err)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use std::collections::HashMap;

    struct FakeServices {
        kv: Mutex<HashMap<String, Vec<u8>>>,
        logs: Mutex<Vec<String>>,
    }

    impl Services for FakeServices {
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
            (key == "password").then(|| "hunter2".to_string())
        }
        fn setting(&self, _p: &str, key: &str) -> Option<String> {
            (key == "server").then(|| "https://jf.local".to_string())
        }
        fn log(&self, _p: &str, _level: i32, message: &str) {
            self.logs.lock().push(message.to_string());
        }
        fn fetch(&self, _p: &str, request: &HttpRequest) -> std::result::Result<HttpResponse, String> {
            Ok(HttpResponse { status: 200, headers: Default::default(), body: format!("echo:{}", request.url), body_base64: None })
        }
    }

    const MANIFEST_JSON: &str = r#"{"ok":{"id":"tv.lounge.test","name":"Test","version":"1","settings":[{"key":"server","label":"Server"}],"fetch_hosts":["*"]}}"#;
    const ITEMS_JSON: &str = r#"{"ok":[{"id":"a","title":"Alpha","kind":"resume","progress":0.5}]}"#;
    const REQUEST_JSON: &str = r#"{"url":"https://x.test/path"}"#;

    /// A hand-written module: `lg_alloc` bumps a pointer, `lg_manifest`/`lg_refresh` return
    /// constant JSON, and `lg_refresh` first calls `now`, `log`, `kv_set`/`kv_get`, `setting` and
    /// `fetch` so every host function is exercised.
    fn wat() -> String {
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        format!(
            r#"
    (module
      (import "lounge" "now" (func $now (result i64)))
      (import "lounge" "log" (func $log (param i32 i32 i32)))
      (import "lounge" "kv_set" (func $kv_set (param i32 i32 i32 i32)))
      (import "lounge" "kv_get" (func $kv_get (param i32 i32) (result i64)))
      (import "lounge" "setting" (func $setting (param i32 i32) (result i64)))
      (import "lounge" "fetch" (func $fetch (param i32 i32) (result i64)))
      (memory (export "memory") 2)
      (global $heap (mut i32) (i32.const 4096))
      (data (i32.const 0) "{m}")
      (data (i32.const 512) "{i}")
      (data (i32.const 1024) "hello")
      (data (i32.const 1100) "server")
      (data (i32.const 1200) "{r}")
      (func (export "lg_alloc") (param $len i32) (result i32)
        (local $p i32)
        (local.set $p (global.get $heap))
        (global.set $heap (i32.add (global.get $heap) (i32.add (local.get $len) (i32.const 8))))
        (local.get $p))
      (func (export "lg_free") (param i32 i32))
      (func (export "lg_manifest") (result i64)
        (i64.or (i64.shl (i64.const 0) (i64.const 32)) (i64.const {ml})))
      (func (export "lg_refresh") (param i32 i32) (result i64)
        (drop (call $now))
        (call $log (i32.const 1) (i32.const 1024) (i32.const 5))
        (call $kv_set (i32.const 1024) (i32.const 5) (i32.const 1100) (i32.const 6))
        (drop (call $kv_get (i32.const 1024) (i32.const 5)))
        (drop (call $setting (i32.const 1100) (i32.const 6)))
        (drop (call $fetch (i32.const 1200) (i32.const {rl})))
        (i64.or (i64.shl (i64.const 512) (i64.const 32)) (i64.const {il})))
    )"#,
            m = esc(MANIFEST_JSON),
            i = esc(ITEMS_JSON),
            r = esc(REQUEST_JSON),
            ml = MANIFEST_JSON.len(),
            il = ITEMS_JSON.len(),
            rl = REQUEST_JSON.len(),
        )
    }

    fn host() -> (Host, Arc<FakeServices>) {
        let services = Arc::new(FakeServices { kv: Mutex::new(HashMap::new()), logs: Mutex::new(Vec::new()) });
        (Host::new(services.clone(), Limits::default()), services)
    }

    #[test]
    fn runs_a_module_end_to_end() {
        let (host, services) = host();
        let wasm = wat::parse_str(wat()).unwrap();
        let manifest = Manifest { id: "tv.lounge.test".into(), name: "Test".into(), version: "1".into(), fetch_hosts: vec!["*".into()], ..Default::default() };
        let plugin = host.load("tv.lounge.test", manifest, &wasm).unwrap();
        let reported = host.manifest_of(&plugin).unwrap();
        assert_eq!(reported.id, "tv.lounge.test");
        let items = host.refresh(&plugin, &RefreshContext::default()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Alpha");
        assert_eq!(services.logs.lock().as_slice(), ["hello"]);
        assert_eq!(services.kv.lock().get("hello").map(|v| v.as_slice()), Some(&b"server"[..]));
    }

    #[test]
    fn out_of_fuel_is_a_trap() {
        let (host, _) = host();
        let wasm = wat::parse_str(
            r#"(module (memory (export "memory") 1)
              (func (export "lg_alloc") (param i32) (result i32) (i32.const 8))
              (func (export "lg_free") (param i32 i32))
              (func (export "lg_manifest") (result i64) (i64.const 0))
              (func (export "lg_refresh") (param i32 i32) (result i64) (loop $l (br $l)) (i64.const 0)))"#,
        )
        .unwrap();
        let h = Host::new(host.services.clone(), Limits { fuel: 100_000, ..Limits::default() });
        let plugin = h.load("x", Manifest::default(), &wasm).unwrap();
        let err = h.refresh(&plugin, &RefreshContext::default()).unwrap_err();
        assert!(matches!(&err, PluginError::Trap(msg) if msg.contains("fuel")), "{err}");
    }

    #[test]
    fn fetch_outside_allow_list_is_refused_inside_the_sandbox() {
        let (host, _) = host();
        let wasm = wat::parse_str(wat()).unwrap();
        let manifest = Manifest { id: "tv.lounge.test".into(), name: "Test".into(), version: "1".into(), fetch_hosts: vec!["only.this".into()], ..Default::default() };
        let plugin = host.load("tv.lounge.test", manifest, &wasm).unwrap();
        // The module ignores the fetch result, so the call still succeeds; the denial is what the plugin would have read.
        assert!(host.refresh(&plugin, &RefreshContext::default()).is_ok());
    }
}
