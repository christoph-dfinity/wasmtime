#![cfg(not(miri))]

//! Host (embedder API) canonical-ABI paths for `memory64` (🐘) components.
//!
//! A pointer pair (`string`/`list`) is laid out in a memory64 component as
//! `[ptr: i64, len: i64]` (16 bytes, 8-aligned) rather than `[ptr: i32, len:
//! i32]` (8 bytes, 4-aligned), and guest pointers are passed as core `i64`s.
//!
//! `layout_*` tests exercise the in-memory layout (wrong even for small
//! pointers if the 32-bit layout is assumed); `truncation_*` tests exercise
//! reading a >4GB pointer (needs a >4GB memory, hence `#[ignore]`d as
//! expensive). The dynamic (`Val`) API and the flat-pointer paths are fixed;
//! the remaining `layout_typed_*` tests are `#[ignore]`d against the
//! outstanding `FIXME(#4311)` in the typed API, whose in-memory layout is still
//! computed from compile-time 32-bit `SIZE32`/`ALIGN32` constants.

use wasmtime::Store;
use wasmtime::component::{Component, Linker, Val};
use wasmtime::{Result, StoreContextMut};
use wasmtime_test_util::component::engine64;

const STRING_RESULT_C64: &str = r#"
(component
  (core module $m
    (memory (export "memory") i64 1)
    (data (i64.const 0) "hello")
    (global $next (mut i64) (i64.const 16))
    (func $realloc (export "realloc")
      (param i64 i64 i64 i64) (result i64)
      (local $ret i64)
      (local.set $ret
        (i64.and (i64.add (global.get $next) (i64.const 7)) (i64.const -8)))
      (global.set $next (i64.add (local.get $ret) (local.get 3)))
      (local.get $ret))
    (func (export "get") (result i64)
      (local $ret i64)
      (local.set $ret
        (call $realloc (i64.const 0) (i64.const 0) (i64.const 8) (i64.const 16)))
      (i64.store          (local.get $ret) (i64.const 0))
      (i64.store offset=8 (local.get $ret) (i64.const 5))
      (local.get $ret))
  )
  (core instance $m (instantiate $m))
  (func (export "get") (result string)
    (canon lift (core func $m "get") (memory $m "memory") (realloc (func $m "realloc"))))
)
"#;

// lift_heap_result + lift_pointer_pair_from_memory (func/typed.rs)
#[test]
#[ignore = "FIXME(#4311): host string-result lifting assumes the 32-bit layout"]
fn layout_typed_string_result() -> Result<()> {
    let engine = engine64();
    let component = Component::new(&engine, STRING_RESULT_C64)?;
    let mut store = Store::new(&engine, ());
    let instance = Linker::new(&engine).instantiate(&mut store, &component)?;
    let func = instance.get_typed_func::<(), (String,)>(&mut store, "get")?;
    let (out,) = func.call(&mut store, ())?;
    assert_eq!(out, "hello");
    Ok(())
}

// load_results (func.rs) + load_flat_pointer_pair (values.rs)
#[test]
fn layout_dynamic_string_result() -> Result<()> {
    let engine = engine64();
    let component = Component::new(&engine, STRING_RESULT_C64)?;
    let mut store = Store::new(&engine, ());
    let instance = Linker::new(&engine).instantiate(&mut store, &component)?;
    let func = instance.get_func(&mut store, "get").unwrap();
    let mut results = [Val::Bool(false)];
    func.call(&mut store, &[], &mut results)?;
    assert_eq!(results[0], Val::String("hello".into()));
    Ok(())
}

const STRING_LEN_C64: &str = r#"
(component
  (core module $m
    (memory (export "memory") i64 1)
    (global $next (mut i64) (i64.const 8))
    (func $realloc (export "realloc")
      (param i64 i64 i64 i64) (result i64)
      (local $ret i64)
      (local.set $ret
        (i64.and (i64.add (global.get $next) (i64.const 7)) (i64.const -8)))
      (global.set $next (i64.add (local.get $ret) (local.get 3)))
      (local.get $ret))
    (func (export "len") (param $ptr i64) (param $len i64) (result i32)
      ;; Sum the bytes so the argument must actually be lowered into memory,
      ;; then return the length (a flat i32 result, avoiding the #4311 lift).
      (local $i i64) (local $sum i32)
      (block $done
        (loop $loop
          (br_if $done (i64.ge_u (local.get $i) (local.get $len)))
          (local.set $sum
            (i32.add (local.get $sum)
              (i32.load8_u (i64.add (local.get $ptr) (local.get $i)))))
          (local.set $i (i64.add (local.get $i) (i64.const 1)))
          (br $loop)))
      (drop (local.get $sum))
      (i32.wrap_i64 (local.get $len)))
  )
  (core instance $m (instantiate $m))
  (func (export "len") (param "a" string) (result u32)
    (canon lift (core func $m "len") (memory $m "memory") (realloc (func $m "realloc"))))
)
"#;

// Isolates the host `realloc` fix: lowering a `string` argument into a 64-bit
// memory calls `realloc` with the `(i64,i64,i64,i64) -> i64` signature. The
// result is a flat `u32`, so this does not depend on the (still unimplemented)
// #4311 result-lifting paths.
#[test]
fn realloc_string_argument() -> Result<()> {
    let engine = engine64();
    let component = Component::new(&engine, STRING_LEN_C64)?;
    let mut store = Store::new(&engine, ());
    let instance = Linker::new(&engine).instantiate(&mut store, &component)?;
    let func = instance.get_typed_func::<(&str,), (u32,)>(&mut store, "len")?;
    for input in ["", "x", "hello", "a longer string needing several bytes"] {
        let (out,) = func.call(&mut store, (input,))?;
        assert_eq!(out, input.len() as u32);
    }
    Ok(())
}

const LIST_RESULT_C64: &str = r#"
(component
  (core module $m
    (memory (export "memory") i64 1)
    (data (i64.const 0) "\01\02\03\04")
    (global $next (mut i64) (i64.const 16))
    (func $realloc (export "realloc")
      (param i64 i64 i64 i64) (result i64)
      (local $ret i64)
      (local.set $ret
        (i64.and (i64.add (global.get $next) (i64.const 7)) (i64.const -8)))
      (global.set $next (i64.add (local.get $ret) (local.get 3)))
      (local.get $ret))
    (func (export "get") (result i64)
      (local $ret i64)
      (local.set $ret
        (call $realloc (i64.const 0) (i64.const 0) (i64.const 8) (i64.const 16)))
      (i64.store          (local.get $ret) (i64.const 0))
      (i64.store offset=8 (local.get $ret) (i64.const 4))
      (local.get $ret))
  )
  (core instance $m (instantiate $m))
  (func (export "get") (result (list u8))
    (canon lift (core func $m "get") (memory $m "memory") (realloc (func $m "realloc"))))
)
"#;

// lift_heap_result + lift_pointer_pair_from_memory, list variant
#[test]
#[ignore = "FIXME(#4311): host list-result lifting assumes the 32-bit layout"]
fn layout_typed_list_result() -> Result<()> {
    let engine = engine64();
    let component = Component::new(&engine, LIST_RESULT_C64)?;
    let mut store = Store::new(&engine, ());
    let instance = Linker::new(&engine).instantiate(&mut store, &component)?;
    let func = instance.get_typed_func::<(), (Vec<u8>,)>(&mut store, "get")?;
    let (out,) = func.call(&mut store, ())?;
    assert_eq!(out, vec![1, 2, 3, 4]);
    Ok(())
}

const STRING_ROUNDTRIP_C64: &str = r#"
(component
  (core module $m
    (memory (export "memory") i64 1)
    (global $next (mut i64) (i64.const 8))
    (func $realloc (export "realloc")
      (param i64 i64 i64 i64) (result i64)
      (local $ret i64)
      (local.set $ret
        (i64.and (i64.add (global.get $next) (i64.const 7)) (i64.const -8)))
      (global.set $next (i64.add (local.get $ret) (local.get 3)))
      (local.get $ret))
    (func (export "roundtrip") (param $ptr i64) (param $len i64) (result i64)
      (local $dst i64)
      (local $ret i64)
      (local.set $dst
        (call $realloc (i64.const 0) (i64.const 0) (i64.const 1) (local.get $len)))
      (memory.copy (local.get $dst) (local.get $ptr) (local.get $len))
      (local.set $ret
        (call $realloc (i64.const 0) (i64.const 0) (i64.const 8) (i64.const 16)))
      (i64.store          (local.get $ret) (local.get $dst))
      (i64.store offset=8 (local.get $ret) (local.get $len))
      (local.get $ret))
  )
  (core instance $m (instantiate $m))
  (func (export "roundtrip") (param "a" string) (result string)
    (canon lift (core func $m "roundtrip") (memory $m "memory") (realloc (func $m "realloc"))))
)
"#;

// Argument lowering (incl. the 64-bit `realloc`) works now; still blocked on
// the #4311 result-lifting path.
#[test]
#[ignore = "FIXME(#4311): host string result lifting assumes the 32-bit layout"]
fn layout_typed_string_roundtrip() -> Result<()> {
    let engine = engine64();
    let component = Component::new(&engine, STRING_ROUNDTRIP_C64)?;
    let mut store = Store::new(&engine, ());
    let instance = Linker::new(&engine).instantiate(&mut store, &component)?;
    let func = instance.get_typed_func::<(&str,), (String,)>(&mut store, "roundtrip")?;
    for input in ["", "x", "hello", "a longer string needing several bytes"] {
        let (out,) = func.call(&mut store, (input,))?;
        assert_eq!(out, input);
    }
    Ok(())
}

// Dynamic Val lowering/lifting counterpart.
#[test]
fn layout_dynamic_string_roundtrip() -> Result<()> {
    let engine = engine64();
    let component = Component::new(&engine, STRING_ROUNDTRIP_C64)?;
    let mut store = Store::new(&engine, ());
    let instance = Linker::new(&engine).instantiate(&mut store, &component)?;
    let func = instance.get_func(&mut store, "roundtrip").unwrap();
    for input in ["", "x", "hello", "a longer string needing several bytes"] {
        let mut results = [Val::Bool(false)];
        func.call(&mut store, &[Val::String(input.into())], &mut results)?;
        assert_eq!(results[0], Val::String(input.into()));
    }
    Ok(())
}

// A >4GB memory: the string lives at 4GB with a decoy at offset 0, so a
// pointer truncated to 32 bits resolves to the decoy.
const HI: u64 = 0x1_0000_0000;

fn host_import_large_string_component() -> String {
    format!(
        r#"
(component
  (import "check" (func $check (param "a" string)))
  (core module $libc
    (memory (export "memory") i64 0x1_0002)
    (data (i64.const 0) "DECOY")
    (data (i64.const {HI}) "hello")
    (func (export "realloc") (param i64 i64 i64 i64) (result i64) (i64.const 0))
  )
  (core instance $libc (instantiate $libc))
  (core func $check
    (canon lower (func $check) (memory $libc "memory") (realloc (func $libc "realloc"))))
  (core module $m
    (import "" "check" (func $check (param i64 i64)))
    (func (export "run") (call $check (i64.const {HI}) (i64.const 5)))
  )
  (core instance $m (instantiate $m (with "" (instance (export "check" (func $check))))))
  (func (export "run") (canon lift (core func $m "run")))
)
"#
    )
}

// lift_pointer_pair_from_flat (func/typed.rs)
#[test]
#[ignore = "expensive: allocates a >4GB linear memory"]
fn truncation_typed_host_import_string() -> Result<()> {
    let engine = engine64();
    let component = Component::new(&engine, &host_import_large_string_component())?;
    let mut store = Store::new(&engine, None::<String>);
    let mut linker = Linker::new(&engine);
    linker.root().func_wrap(
        "check",
        |mut cx: StoreContextMut<Option<String>>, (arg,): (String,)| {
            *cx.data_mut() = Some(arg);
            Ok(())
        },
    )?;
    let instance = linker.instantiate(&mut store, &component)?;
    let func = instance.get_typed_func::<(), ()>(&mut store, "run")?;
    func.call(&mut store, ())?;
    assert_eq!(store.data().as_deref(), Some("hello"));
    Ok(())
}

// lift_flat_pointer_pair (values.rs)
#[test]
#[ignore = "expensive: allocates a >4GB linear memory"]
fn truncation_dynamic_host_import_string() -> Result<()> {
    let engine = engine64();
    let component = Component::new(&engine, &host_import_large_string_component())?;
    let mut store = Store::new(&engine, None::<String>);
    let mut linker = Linker::new(&engine);
    linker.root().func_new(
        "check",
        |mut cx: StoreContextMut<Option<String>>, _, args, _results| {
            let Val::String(s) = &args[0] else {
                panic!("expected string, got {:?}", args[0]);
            };
            *cx.data_mut() = Some(s.clone());
            Ok(())
        },
    )?;
    let instance = linker.instantiate(&mut store, &component)?;
    let func = instance.get_func(&mut store, "run").unwrap();
    func.call(&mut store, &[], &mut [])?;
    assert_eq!(store.data().as_deref(), Some("hello"));
    Ok(())
}
