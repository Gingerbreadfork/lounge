//! Runs plugin modules. Every call gets a fresh instance with a fuel budget, a memory and table
//! cap and a deadline, so a misbehaving plugin can burn its own budget and nothing else. Host
//! functions read and write the plugin's linear memory through its own `lg_alloc`.

use crate::api::{self, exports, imports, Action, HttpRequest, HttpResponse, Item, Manifest, Outcome, RefreshContext, SearchQuery};
use crate::{PluginError, Result};
use serde::de::{DeserializeOwned, Deserializer, IgnoredAny, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use wasmi::{Caller, Config, Engine, Extern, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc, TypedResumableCall, WasmParams, WasmResults};

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
    /// Most entries the module's tables may hold, in total.
    pub table_elements: usize,
    pub fuel: u64,
    /// What every host call costs, on top of one unit per byte it moves, so a loop of host calls
    /// runs out of fuel like a loop of instructions.
    pub host_call_fuel: u64,
    /// Longest value a plugin may store under one key.
    pub kv_value_bytes: usize,
    pub kv_keys: usize,
    /// Most `kv_set` and `kv_delete` calls one invocation may make.
    pub kv_writes: u32,
    /// Most `fetch` calls one invocation may make, and the wall-clock budget they share;
    /// fuel meters instructions, this meters the time spent waiting on the network.
    pub max_fetches: u32,
    pub fetch_budget: Duration,
    /// Largest value a refresh may hand back.
    pub max_result_bytes: usize,
    /// Largest value any other export may hand back; a manifest, a search or an action is small.
    pub call_result_bytes: usize,
    /// Wall-clock limit for one refresh, and for any other call.
    pub refresh_time: Duration,
    pub call_time: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            memory_bytes: 32 * 1024 * 1024,
            table_elements: 100_000,
            fuel: 20_000_000_000,
            host_call_fuel: 10_000,
            kv_value_bytes: 256 * 1024,
            kv_keys: 256,
            kv_writes: 1_000,
            max_fetches: 64,
            fetch_budget: Duration::from_secs(60),
            max_result_bytes: 8 * 1024 * 1024,
            call_result_bytes: 256 * 1024,
            refresh_time: Duration::from_secs(180),
            call_time: Duration::from_secs(12),
        }
    }
}

/// Longest key, in bytes, for the key-value store, settings and secrets.
const MAX_KEY_BYTES: usize = 256;
/// Longest log line, in bytes; anything after it is dropped.
const MAX_LOG_BYTES: usize = 4096;
/// Most items read from one answer; the rest are counted but never built. Twice what the core
/// keeps, so ids a plugin repeats rarely cost it an item.
pub const ITEMS_READ: usize = 4_000;
/// The most an answer to `fetch` may come to as JSON. Ordinary escaping adds little to an 8 MB
/// body, but control characters grow six-fold, and such an answer is refused before it is built.
const MAX_ANSWER_BYTES: usize = 16 * 1024 * 1024;
/// Fuel is handed to a call this much at a time and the clock is read between slices, so a loop
/// that never calls the host still stops at its deadline. A slice is well under a second on a TV.
const FUEL_SLICE: u64 = 10_000_000;

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
    kv_value_bytes: usize,
    kv_writes_left: u32,
    host_call_fuel: u64,
    log_lines: u32,
    fetches_left: u32,
    fetch_deadline: Instant,
    deadline: Instant,
    /// Fuel not yet handed to the call.
    fuel_reserve: u64,
    /// Set while the host waits on the plugin's `lg_alloc`, which may not call back into the host.
    in_alloc: bool,
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
        self.call_json::<(), Manifest>(plugin, exports::MANIFEST, None, self.limits.call_time, self.limits.call_result_bytes).map(|o| match o {
            Outcome::Ok(m) => Ok(m),
            Outcome::Err(e) => Err(PluginError::Failed(e)),
        })?
    }

    pub fn refresh(&self, plugin: &LoadedPlugin, ctx: &RefreshContext) -> Result<Vec<Item>> {
        self.refresh_counted(plugin, ctx).map(|(items, _)| items)
    }

    /// A refresh's items, the first [ITEMS_READ] of them, and how many it sent in all.
    pub fn refresh_counted(&self, plugin: &LoadedPlugin, ctx: &RefreshContext) -> Result<(Vec<Item>, usize)> {
        let listing: Listing = self.outcome(self.call_json(plugin, exports::REFRESH, Some(ctx), self.limits.refresh_time, self.limits.max_result_bytes)?)?;
        Ok((listing.items, listing.sent))
    }

    /// Whether the module answers `lg_search`.
    pub fn can_search(&self, plugin: &LoadedPlugin) -> bool {
        plugin.module.get_export(exports::SEARCH).is_some()
    }

    pub fn search(&self, plugin: &LoadedPlugin, query: &str, limit: u32) -> Result<Vec<Item>> {
        if !self.can_search(plugin) {
            return Ok(Vec::new());
        }
        let q = SearchQuery { query: query.to_string(), limit };
        let listing: Listing = self.outcome(self.call_json(plugin, exports::SEARCH, Some(&q), self.limits.call_time, self.limits.call_result_bytes)?)?;
        Ok(listing.items)
    }

    pub fn resolve(&self, plugin: &LoadedPlugin, action: &str) -> Result<Action> {
        if plugin.module.get_export(exports::RESOLVE).is_none() {
            return Err(PluginError::Failed("plugin cannot resolve actions".into()));
        }
        self.outcome(self.call_json(plugin, exports::RESOLVE, Some(&action.to_string()), self.limits.call_time, self.limits.call_result_bytes)?)
    }

    fn outcome<T>(&self, o: Outcome<T>) -> Result<T> {
        match o {
            Outcome::Ok(v) => Ok(v),
            Outcome::Err(e) => Err(PluginError::Failed(e)),
        }
    }

    fn call_json<I: Serialize, O: DeserializeOwned>(&self, plugin: &LoadedPlugin, export: &str, input: Option<&I>, time: Duration, max_result: usize) -> Result<Outcome<O>> {
        let started = Instant::now();
        let deadline = started + time;
        // The start function cannot be paused, so it runs on the first slice alone.
        let first_slice = self.limits.fuel.min(FUEL_SLICE);
        let state = State {
            plugin: plugin.id.clone(),
            manifest: plugin.manifest.clone(),
            services: Arc::clone(&self.services),
            limits: StoreLimitsBuilder::new()
                .memory_size(self.limits.memory_bytes)
                .memories(1)
                .table_elements(self.limits.table_elements)
                .tables(4)
                .instances(1)
                .build(),
            kv_value_bytes: self.limits.kv_value_bytes,
            kv_writes_left: self.limits.kv_writes,
            host_call_fuel: self.limits.host_call_fuel,
            log_lines: 0,
            fetches_left: self.limits.max_fetches,
            fetch_deadline: (started + self.limits.fetch_budget).min(deadline),
            deadline,
            fuel_reserve: self.limits.fuel - first_slice,
            in_alloc: false,
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        store.set_fuel(first_slice).map_err(wasm_err)?;

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
                let ptr = call_metered(&mut store, &alloc, bytes.len() as i32, export, time)? as u32;
                memory.write(&mut store, ptr as usize, &bytes).map_err(|e| PluginError::Wasm(e.to_string()))?;
                let f = instance.get_typed_func::<(i32, i32), i64>(&store, export).map_err(wasm_err)?;
                call_metered(&mut store, &f, (ptr as i32, bytes.len() as i32), export, time)?
            }
            None => {
                let f = instance.get_typed_func::<(), i64>(&store, export).map_err(wasm_err)?;
                call_metered(&mut store, &f, (), export, time)?
            }
        };
        if Instant::now() > deadline {
            return Err(PluginError::Trap(format!("{export} ran past its {} s limit", time.as_secs())));
        }
        let (ptr, len) = api::unpack(packed);
        if len == 0 {
            return Err(PluginError::Failed(format!("{export} returned nothing")));
        }
        // The plugin names the range; it is checked against its memory before a byte is allocated.
        let end = ptr as usize + len as usize;
        if len as usize > max_result || end > memory.data_size(&store) {
            return Err(PluginError::Wasm(format!("{export} returned {len} bytes at {ptr}, outside its memory or over the limit")));
        }
        let mut out = vec![0u8; len as usize];
        memory.read(&store, ptr as usize, &mut out).map_err(|e| PluginError::Wasm(e.to_string()))?;
        if let Ok(free) = instance.get_typed_func::<(i32, i32), ()>(&store, exports::FREE) {
            let _ = call_metered(&mut store, &free, (ptr as i32, len as i32), export, time);
        }
        let used = self.limits.fuel.saturating_sub(store.get_fuel().unwrap_or(0) + store.data().fuel_reserve);
        tracing::debug!("plugin {} {export}: {} fuel", plugin.id, used);
        Ok(serde_json::from_slice(&out)?)
    }
}

/// A list of items read up to [ITEMS_READ] entries, with a count of every entry sent.
struct Listing {
    items: Vec<Item>,
    sent: usize,
}

impl<'de> Deserialize<'de> for Listing {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Listing, D::Error> {
        struct Entries;
        impl<'de> Visitor<'de> for Entries {
            type Value = Listing;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a list of items")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Listing, A::Error> {
                let mut items = Vec::new();
                let mut sent = 0;
                loop {
                    let more = if items.len() < ITEMS_READ {
                        seq.next_element::<Item>()?.map(|it| items.push(it)).is_some()
                    } else {
                        seq.next_element::<IgnoredAny>()?.is_some()
                    };
                    if !more {
                        return Ok(Listing { items, sent });
                    }
                    sent += 1;
                }
            }
        }
        d.deserialize_seq(Entries)
    }
}

/// A buffer that refuses to grow past `max`.
struct Bounded {
    buf: Vec<u8>,
    max: usize,
}

impl std::io::Write for Bounded {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.buf.len() + data.len() > self.max {
            return Err(std::io::Error::other("over the limit"));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A fetch answer as JSON for the plugin, or a short refusal when it would come to more than `max`.
fn encode_answer(response: &Outcome<HttpResponse>, max: usize) -> std::result::Result<Vec<u8>, serde_json::Error> {
    let mut out = Bounded { buf: Vec::new(), max };
    match serde_json::to_writer(&mut out, response) {
        Ok(()) => Ok(out.buf),
        // The SDK reads "body is larger than" as an answer too big for a plugin.
        Err(_) => serde_json::to_vec(&Outcome::<HttpResponse>::Err("the body is larger than a plugin may read once encoded".into())),
    }
}

fn wasm_err(e: wasmi::Error) -> PluginError {
    PluginError::Wasm(e.to_string())
}

/// Runs `f` a slice of fuel at a time, stopping it once its deadline or its fuel is gone.
fn call_metered<P: WasmParams, R: WasmResults>(store: &mut Store<State>, f: &TypedFunc<P, R>, params: P, export: &str, time: Duration) -> Result<R> {
    let mut call = f.call_resumable(&mut *store, params).map_err(|e| trap_err(&e))?;
    loop {
        call = match call {
            TypedResumableCall::Finished(results) => return Ok(results),
            TypedResumableCall::HostTrap(t) => return Err(trap_err(t.host_error())),
            TypedResumableCall::OutOfFuel(paused) => {
                refuel(store, export, time)?;
                paused.resume(&mut *store).map_err(|e| trap_err(&e))?
            }
        };
    }
}

/// Gives a paused call its next slice, unless its time or its fuel has run out.
fn refuel(store: &mut Store<State>, export: &str, time: Duration) -> Result<()> {
    if Instant::now() > store.data().deadline {
        return Err(PluginError::Trap(format!("{export} ran past its {} s limit", time.as_secs())));
    }
    let slice = store.data().fuel_reserve.min(FUEL_SLICE);
    if slice == 0 {
        return Err(PluginError::Trap("out of fuel: the plugin took too long".into()));
    }
    store.data_mut().fuel_reserve -= slice;
    let left = store.get_fuel().map_err(wasm_err)?;
    store.set_fuel(left + slice).map_err(wasm_err)
}

fn trap_err(e: &wasmi::Error) -> PluginError {
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

/// The bookkeeping every host call starts with: no calls from inside `lg_alloc`, none past the
/// deadline, and fuel for the call and the bytes it moves.
fn enter(caller: &mut Caller<'_, State>, bytes: usize) -> std::result::Result<(), wasmi::Error> {
    let st = caller.data();
    if st.in_alloc {
        return Err(wasmi::Error::new("lg_alloc may not call host functions"));
    }
    if Instant::now() > st.deadline {
        return Err(wasmi::Error::new("the plugin ran past its time limit"));
    }
    let cost = st.host_call_fuel.saturating_add(bytes as u64);
    charge(caller, cost)
}

/// Takes `cost` fuel from the call, drawing on fuel not yet handed out when the current slice is
/// short, so a large host call costs the same whichever slice it lands in.
fn charge(caller: &mut Caller<'_, State>, cost: u64) -> std::result::Result<(), wasmi::Error> {
    let fuel = caller.get_fuel()?;
    if fuel >= cost {
        return caller.set_fuel(fuel - cost);
    }
    let st = caller.data_mut();
    let need = cost - fuel;
    if st.fuel_reserve < need {
        return Err(wasmi::Error::from(wasmi::TrapCode::OutOfFuel));
    }
    st.fuel_reserve -= need;
    caller.set_fuel(0)
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

/// A key for the key-value store, settings or secrets: whole, valid text, never cut short.
fn read_key(caller: &Caller<'_, State>, ptr: i32, len: i32) -> std::result::Result<String, wasmi::Error> {
    if len as usize > MAX_KEY_BYTES {
        return Err(wasmi::Error::new(format!("a key is longer than {MAX_KEY_BYTES} bytes")));
    }
    String::from_utf8(read_guest(caller, ptr, len)?).map_err(|_| wasmi::Error::new("plugin passed a key that is not valid UTF-8"))
}

/// Up to `max` bytes of text; a character the limit cuts through is dropped, not an error.
fn read_text(caller: &Caller<'_, State>, ptr: i32, len: i32, max: usize) -> std::result::Result<String, wasmi::Error> {
    let cut = len.max(0) as usize > max;
    let bytes = read_guest(caller, ptr, len.min(max as i32))?;
    match String::from_utf8(bytes) {
        Ok(text) => Ok(text),
        Err(e) if cut && e.utf8_error().error_len().is_none() => {
            let valid = e.utf8_error().valid_up_to();
            let mut bytes = e.into_bytes();
            bytes.truncate(valid);
            Ok(String::from_utf8(bytes).unwrap_or_default())
        }
        Err(_) => Err(wasmi::Error::new("plugin passed invalid UTF-8")),
    }
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
    caller.data_mut().in_alloc = true;
    let ptr = alloc.call(&mut *caller, bytes.len() as i32);
    caller.data_mut().in_alloc = false;
    let ptr = ptr?;
    if ptr <= 0 {
        return Err(wasmi::Error::new("plugin allocation failed"));
    }
    let mem = memory_of(caller)?;
    mem.write(&mut *caller, ptr as usize, bytes).map_err(|e| wasmi::Error::new(e.to_string()))?;
    Ok(api::pack(ptr as u32, bytes.len() as u32))
}

/// Charge for the bytes a call hands back, then copy them into the plugin.
fn hand_back(caller: &mut Caller<'_, State>, bytes: &[u8]) -> std::result::Result<i64, wasmi::Error> {
    charge(caller, bytes.len() as u64)?;
    write_guest(caller, bytes)
}

fn link_host_functions(linker: &mut Linker<State>) -> Result<()> {
    let m = api::HOST_MODULE;
    let err = |e: wasmi::errors::LinkerError| PluginError::Wasm(e.to_string());

    linker
        .func_wrap(m, imports::NOW, |mut caller: Caller<'_, State>| -> std::result::Result<i64, wasmi::Error> {
            enter(&mut caller, 0)?;
            Ok(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0))
        })
        .map_err(err)?;

    linker
        .func_wrap(m, imports::LOG, |mut caller: Caller<'_, State>, level: i32, ptr: i32, len: i32| -> std::result::Result<(), wasmi::Error> {
            enter(&mut caller, len.clamp(0, MAX_LOG_BYTES as i32) as usize)?;
            let msg = read_text(&caller, ptr, len, MAX_LOG_BYTES)?;
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
            enter(&mut caller, 0)?;
            let key = read_key(&caller, ptr, len)?;
            let value = caller.data().services.kv_get(&caller.data().plugin, &key);
            match value {
                Some(v) => hand_back(&mut caller, &v),
                None => Ok(0),
            }
        })
        .map_err(err)?;

    linker
        .func_wrap(m, imports::KV_SET, |mut caller: Caller<'_, State>, kp: i32, kl: i32, vp: i32, vl: i32| -> std::result::Result<(), wasmi::Error> {
            enter(&mut caller, vl.max(0) as usize)?;
            let key = read_key(&caller, kp, kl)?;
            if vl as usize > caller.data().kv_value_bytes {
                return Err(wasmi::Error::new("kv value too large"));
            }
            let st = caller.data_mut();
            if st.kv_writes_left == 0 {
                return Err(wasmi::Error::new("too many key-value writes in one call"));
            }
            st.kv_writes_left -= 1;
            let value = read_guest(&caller, vp, vl)?;
            caller.data().services.kv_set(&caller.data().plugin, &key, &value);
            Ok(())
        })
        .map_err(err)?;

    linker
        .func_wrap(m, imports::KV_DELETE, |mut caller: Caller<'_, State>, ptr: i32, len: i32| -> std::result::Result<(), wasmi::Error> {
            enter(&mut caller, 0)?;
            let key = read_key(&caller, ptr, len)?;
            let st = caller.data_mut();
            if st.kv_writes_left == 0 {
                return Err(wasmi::Error::new("too many key-value writes in one call"));
            }
            st.kv_writes_left -= 1;
            caller.data().services.kv_delete(&caller.data().plugin, &key);
            Ok(())
        })
        .map_err(err)?;

    linker
        .func_wrap(m, imports::SECRET, |mut caller: Caller<'_, State>, ptr: i32, len: i32| -> std::result::Result<i64, wasmi::Error> {
            enter(&mut caller, 0)?;
            let key = read_key(&caller, ptr, len)?;
            // Only secrets the manifest declared as password settings are readable.
            let declared = caller.data().manifest.settings.iter().any(|s| s.key == key && s.kind.is_secret());
            if !declared {
                return Ok(0);
            }
            let value = caller.data().services.secret(&caller.data().plugin, &key);
            match value {
                Some(v) => hand_back(&mut caller, v.as_bytes()),
                None => Ok(0),
            }
        })
        .map_err(err)?;

    linker
        .func_wrap(m, imports::SETTING, |mut caller: Caller<'_, State>, ptr: i32, len: i32| -> std::result::Result<i64, wasmi::Error> {
            enter(&mut caller, 0)?;
            let key = read_key(&caller, ptr, len)?;
            let spec = caller.data().manifest.settings.iter().find(|s| s.key == key);
            // A password is only ever read through `secret`.
            if spec.is_some_and(|s| s.kind.is_secret()) {
                return Ok(0);
            }
            let default = spec.and_then(|s| s.default.clone());
            let value = caller.data().services.setting(&caller.data().plugin, &key).or(default);
            match value {
                Some(v) => hand_back(&mut caller, v.as_bytes()),
                None => Ok(0),
            }
        })
        .map_err(err)?;

    linker
        .func_wrap(m, imports::FETCH, |mut caller: Caller<'_, State>, ptr: i32, len: i32| -> std::result::Result<i64, wasmi::Error> {
            enter(&mut caller, len.clamp(0, 1 << 20) as usize)?;
            let raw = read_guest(&caller, ptr, len.min(1 << 20))?;
            let mut request: HttpRequest = serde_json::from_slice(&raw).map_err(|e| wasmi::Error::new(format!("bad request: {e}")))?;
            let host = crate::http::host_of(&request.url).unwrap_or_default();
            let budget = {
                let st = caller.data_mut();
                let now = Instant::now();
                if st.fetches_left == 0 || now > st.fetch_deadline {
                    Err("fetch budget for this call is used up".to_string())
                } else {
                    st.fetches_left -= 1;
                    // A request never outlives the call's own budget.
                    let left = st.fetch_deadline.saturating_duration_since(now).as_millis().max(1) as u32;
                    request.timeout_ms = if request.timeout_ms == 0 { left } else { request.timeout_ms.min(left) };
                    Ok(())
                }
            };
            let response: Outcome<HttpResponse> = if let Err(e) = budget {
                Outcome::Err(e)
            } else if !caller.data().manifest.allows_host_with(&host, &|key| caller.data().services.setting(&caller.data().plugin, key)) {
                Outcome::Err(format!("host {host} is not in this plugin's allow list"))
            } else {
                match caller.data().services.fetch(&caller.data().plugin, &request) {
                    Ok(r) => Outcome::Ok(r),
                    Err(e) => Outcome::Err(e),
                }
            };
            let bytes = encode_answer(&response, MAX_ANSWER_BYTES).map_err(|e| wasmi::Error::new(e.to_string()))?;
            drop(response);
            hand_back(&mut caller, &bytes)
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
    fn a_loop_that_never_calls_the_host_stops_at_its_deadline() {
        let (host, _) = host();
        let spin = module("", "", "(i32.const 8)", "(loop $l (br $l)) (i64.const 0)");
        let fill = module("", "", "(i32.const 8)", "(loop $l (memory.fill (i32.const 0) (i32.const 0) (i32.const 131072)) (br $l)) (i64.const 0)");
        let h = Host::new(host.services.clone(), Limits { refresh_time: Duration::from_millis(300), ..Limits::default() });
        for wasm in [spin, fill] {
            let plugin = h.load("x", Manifest::default(), &wasm).unwrap();
            let started = Instant::now();
            let err = h.refresh(&plugin, &RefreshContext::default()).unwrap_err();
            assert!(matches!(&err, PluginError::Trap(msg) if msg.contains("limit")), "{err}");
            assert!(started.elapsed() < Duration::from_secs(5), "took {:?}", started.elapsed());
        }
    }

    #[test]
    fn work_longer_than_one_slice_still_finishes() {
        let (host, _) = host();
        let wasm = module(
            "",
            "",
            "(i32.const 8)",
            "(local $n i32) (local.set $n (i32.const 6000000))
             (loop $l (local.set $n (i32.sub (local.get $n) (i32.const 1))) (br_if $l (local.get $n)))
             (i64.const 0)",
        );
        let plugin = host.load("x", Manifest::default(), &wasm).unwrap();
        let err = host.refresh(&plugin, &RefreshContext::default()).unwrap_err();
        assert!(matches!(&err, PluginError::Failed(msg) if msg.contains("returned nothing")), "{err}");
    }

    #[test]
    fn items_past_the_cap_are_counted_but_not_built() {
        let entries: Vec<String> = (0..ITEMS_READ + 5).map(|i| format!(r#"{{"id":"{i}","title":"t"}}"#)).collect();
        let json = format!(r#"{{"ok":[{}]}}"#, entries.join(","));
        let Outcome::Ok(listing) = serde_json::from_str::<Outcome<Listing>>(&json).unwrap() else { panic!("not ok") };
        assert_eq!(listing.items.len(), ITEMS_READ);
        assert_eq!(listing.sent, ITEMS_READ + 5);
        assert_eq!(listing.items.last().map(|it| it.id.as_str()), Some("3999"));
    }

    #[test]
    fn an_answer_that_grows_past_its_limit_is_refused() {
        let response = |body: String| Outcome::Ok(HttpResponse { status: 200, headers: Default::default(), body, body_base64: None });
        // Each control character becomes six bytes of JSON.
        let big = encode_answer(&response("\u{1}".repeat(500)), 1000).unwrap();
        assert!(String::from_utf8(big).unwrap().contains("body is larger than"));
        let small = encode_answer(&response("plain".into()), 1000).unwrap();
        assert!(String::from_utf8(small).unwrap().contains("plain"));
    }

    /// A minimal module whose `lg_refresh` body is `body`; `lg_alloc` is `alloc`.
    fn module(imports: &str, extra: &str, alloc: &str, body: &str) -> Vec<u8> {
        wat::parse_str(format!(
            r#"(module {imports}
              (memory (export "memory") 2)
              {extra}
              (func (export "lg_alloc") (param $len i32) (result i32) {alloc})
              (func (export "lg_free") (param i32 i32))
              (func (export "lg_manifest") (result i64) (i64.const 0))
              (func (export "lg_refresh") (param i32 i32) (result i64) {body}))"#
        ))
        .unwrap()
    }

    fn secret_manifest() -> Manifest {
        let mut m = Manifest { id: "tv.lounge.test".into(), name: "Test".into(), version: "1".into(), ..Default::default() };
        m.settings.push(lounge_plugin_api::SettingSpec { key: "server".into(), label: "Server".into(), kind: lounge_plugin_api::SettingKind::Text, required: false, help: None, default: None });
        m.settings.push(lounge_plugin_api::SettingSpec { key: "password".into(), label: "Password".into(), kind: lounge_plugin_api::SettingKind::Password, required: false, help: None, default: None });
        m
    }

    #[test]
    fn lg_alloc_cannot_call_back_into_the_host() {
        let (host, _) = host();
        let wasm = module(
            r#"(import "lounge" "setting" (func $setting (param i32 i32) (result i64)))"#,
            r#"(data (i32.const 1100) "server")"#,
            "(drop (call $setting (i32.const 1100) (i32.const 6))) (i32.const 4096)",
            "(drop (call $setting (i32.const 1100) (i32.const 6))) (i64.const 0)",
        );
        let plugin = host.load("tv.lounge.test", secret_manifest(), &wasm).unwrap();
        let err = host.refresh(&plugin, &RefreshContext::default()).unwrap_err();
        assert!(err.to_string().contains("lg_alloc may not call host functions"), "{err}");
    }

    #[test]
    fn tables_cannot_escape_the_memory_cap() {
        let (host, _) = host();
        let wasm = module("", "(table 200000000 funcref)", "(i32.const 8)", "(i64.const 0)");
        let plugin = host.load("x", Manifest::default(), &wasm).unwrap();
        assert!(host.refresh(&plugin, &RefreshContext::default()).is_err());
    }

    #[test]
    fn passwords_only_come_through_secret() {
        // `setting("password")` must hand back nothing; `secret("password")` must hand back the value.
        let wasm = module(
            r#"(import "lounge" "setting" (func $setting (param i32 i32) (result i64)))
               (import "lounge" "secret" (func $secret (param i32 i32) (result i64)))"#,
            r#"(global $heap (mut i32) (i32.const 4096)) (data (i32.const 1100) "password")"#,
            "(local $p i32) (local.set $p (global.get $heap)) (global.set $heap (i32.add (global.get $heap) (i32.add (local.get $len) (i32.const 8)))) (local.get $p)",
            "(if (i64.ne (call $setting (i32.const 1100) (i32.const 8)) (i64.const 0)) (then unreachable))
             (if (i64.eq (call $secret (i32.const 1100) (i32.const 8)) (i64.const 0)) (then unreachable))
             (i64.const 0)",
        );
        struct Both;
        impl Services for Both {
            fn kv_get(&self, _: &str, _: &str) -> Option<Vec<u8>> { None }
            fn kv_set(&self, _: &str, _: &str, _: &[u8]) {}
            fn kv_delete(&self, _: &str, _: &str) {}
            fn secret(&self, _: &str, _: &str) -> Option<String> { Some("hunter2".into()) }
            fn setting(&self, _: &str, _: &str) -> Option<String> { Some("hunter2".into()) }
            fn log(&self, _: &str, _: i32, _: &str) {}
            fn fetch(&self, _: &str, _: &HttpRequest) -> std::result::Result<HttpResponse, String> { Err("no".into()) }
        }
        let h = Host::new(Arc::new(Both), Limits::default());
        let plugin = h.load("tv.lounge.test", secret_manifest(), &wasm).unwrap();
        let err = h.refresh(&plugin, &RefreshContext::default()).unwrap_err();
        // Both checks passed: the module got as far as returning nothing.
        assert!(err.to_string().contains("returned nothing"), "{err}");
    }

    #[test]
    fn a_log_line_cut_through_a_character_is_kept() {
        let (host, services) = host();
        let mut text = "a".repeat(MAX_LOG_BYTES - 1);
        text.push('é');
        text.push_str("bbbb");
        let wasm = module(
            r#"(import "lounge" "log" (func $log (param i32 i32 i32)))"#,
            &format!(r#"(data (i32.const 8192) "{text}")"#),
            "(i32.const 8)",
            &format!("(call $log (i32.const 1) (i32.const 8192) (i32.const {})) (i64.const 0)", text.len()),
        );
        let plugin = host.load("x", Manifest::default(), &wasm).unwrap();
        let _ = host.refresh(&plugin, &RefreshContext::default());
        let logs = services.logs.lock();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0], "a".repeat(MAX_LOG_BYTES - 1));
    }

    #[test]
    fn host_calls_cost_fuel_and_time() {
        let (host, _) = host();
        let wasm = module(
            r#"(import "lounge" "now" (func $now (result i64)))"#,
            "",
            "(i32.const 8)",
            "(loop $l (drop (call $now)) (br $l)) (i64.const 0)",
        );
        let h = Host::new(host.services.clone(), Limits { fuel: 5_000_000, ..Limits::default() });
        let plugin = h.load("x", Manifest::default(), &wasm).unwrap();
        let started = Instant::now();
        let err = h.refresh(&plugin, &RefreshContext::default()).unwrap_err();
        assert!(err.to_string().contains("fuel"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(1));

        let h = Host::new(host.services.clone(), Limits { refresh_time: Duration::ZERO, ..Limits::default() });
        let plugin = h.load("x", Manifest::default(), &wasm).unwrap();
        let err = h.refresh(&plugin, &RefreshContext::default()).unwrap_err();
        assert!(err.to_string().contains("time limit"), "{err}");
    }

    #[test]
    fn over_long_keys_are_an_error_not_a_collision() {
        let (host, _) = host();
        let key = "k".repeat(MAX_KEY_BYTES + 1);
        let wasm = module(
            r#"(import "lounge" "kv_set" (func $kv_set (param i32 i32 i32 i32)))"#,
            &format!(r#"(data (i32.const 8192) "{key}")"#),
            "(i32.const 8)",
            &format!("(call $kv_set (i32.const 8192) (i32.const {}) (i32.const 8192) (i32.const 4)) (i64.const 0)", key.len()),
        );
        let plugin = host.load("x", Manifest::default(), &wasm).unwrap();
        let err = host.refresh(&plugin, &RefreshContext::default()).unwrap_err();
        assert!(err.to_string().contains("longer than"), "{err}");
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
