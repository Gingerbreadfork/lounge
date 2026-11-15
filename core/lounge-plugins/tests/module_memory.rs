//! A loaded plugin gives all its memory back when it is let go, however often it is loaded again.

use lounge_plugins::host::{Host, Limits, Services};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Counts the bytes this test binary has allocated and not yet freed.
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        LIVE.fetch_add(new_size, Ordering::Relaxed);
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

struct Nothing;

impl Services for Nothing {
    fn kv_get(&self, _: &str, _: &str) -> Option<Vec<u8>> { None }
    fn kv_set(&self, _: &str, _: &str, _: &[u8], _: usize) -> bool { true }
    fn kv_delete(&self, _: &str, _: &str) {}
    fn secret(&self, _: &str, _: &str) -> Option<String> { None }
    fn setting(&self, _: &str, _: &str) -> Option<String> { None }
    fn log(&self, _: &str, _: i32, _: &str) {}
    fn fetch(&self, _: &str, _: &lounge_plugin_api::HttpRequest) -> Result<lounge_plugin_api::HttpResponse, String> {
        Err("offline".into())
    }
}

/// A module with enough functions for its size to show.
fn module() -> Vec<u8> {
    let mut text = String::from(
        r#"(module (memory (export "memory") 1)
          (func (export "lg_alloc") (param i32) (result i32) (i32.const 1024))
          (func (export "lg_free") (param i32 i32))
          (func (export "lg_manifest") (result i64) (i64.const 0))
          (func (export "lg_refresh") (param i32 i32) (result i64) (i64.const 0))"#,
    );
    for n in 0..400 {
        text.push_str(&format!("(func (export \"f{n}\") (param i32) (result i32) "));
        for _ in 0..40 {
            text.push_str("(local.set 0 (i32.add (local.get 0) (i32.const 7)))");
        }
        text.push_str("(local.get 0))");
    }
    text.push(')');
    wat::parse_str(text).unwrap()
}

#[test]
fn a_plugin_let_go_gives_its_memory_back() {
    let wasm = module();
    let host = Host::new(Arc::new(Nothing), Limits::default());
    let start = LIVE.load(Ordering::Relaxed);
    let plugin = host.load("tv.example.test", Default::default(), &wasm).unwrap();
    let _ = host.refresh(&plugin, &Default::default());
    let loaded = LIVE.load(Ordering::Relaxed) - start;
    assert!(loaded > wasm.len() / 2, "a loaded module holds {loaded} bytes");
    drop(plugin);
    for _ in 0..20 {
        let plugin = host.load("tv.example.test", Default::default(), &wasm).unwrap();
        let _ = host.refresh(&plugin, &Default::default());
    }
    let left = LIVE.load(Ordering::Relaxed).saturating_sub(start);
    assert!(left < loaded / 2, "{left} bytes are still held after every plugin was let go; one holds {loaded}");
}
