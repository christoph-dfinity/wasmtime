;;! component_model_memory64 = true
;;! memory64 = true
;;! multi_memory = true
;;! bulk_memory = true

;; Exercise a fused adapter that passes a `(list u8)` across the boundary
;; between a 64-bit component (core memory is `i64`) and a 32-bit component
;; (core memory is `i32`).
;;
;; The top-level `roundtrip` export is lifted out of the 32-bit component. When
;; invoked, the list travels:
;;
;;   host -> (lift into c32's i32 memory)
;;        -> (lower out of c32's i32 memory into the canonical ABI)
;;        -> (lift into c64's i64 memory) -> c64 copies the bytes
;;        -> (lower back out of c64's i64 memory)
;;        -> (lift back into c32's i32 memory)
;;        -> host
;;
;; so the list is copied through both a 32-bit and a 64-bit linear memory in
;; both the argument and result directions.

(component
  ;; The 64-bit "backend" component. Its core memory is declared with the
  ;; `i64` index type, so the canonical ABI flattens `(list u8)` into a pair of
  ;; `i64`s and `realloc` uses `i64` offsets.
  (component $c64
    (core module $m
      (memory (export "memory") i64 1)

      ;; Simple bump allocator that hands out 8-byte-aligned regions. `realloc`
      ;; is invoked both by the canonical ABI (to allocate the incoming list and
      ;; the returned list) and by `roundtrip` below.
      (global $next (mut i64) (i64.const 8))
      (func $realloc (export "realloc")
        (param $old i64) (param $old_sz i64) (param $align i64) (param $new_sz i64)
        (result i64)
        (local $ret i64)
        (local.set $ret
          (i64.and
            (i64.add (global.get $next) (i64.const 7))
            (i64.const -8)))
        (global.set $next (i64.add (local.get $ret) (local.get $new_sz)))
        (local.get $ret))

      ;; Receives `(ptr, len)` describing the incoming list, copies the bytes
      ;; into a freshly allocated buffer, and returns a pointer to a
      ;; `[ptr:i64, len:i64]` structure describing the copy (indirect return of
      ;; a `(list u8)`).
      (func (export "roundtrip") (param $ptr i64) (param $len i64) (result i64)
        (local $dst i64)
        (local $ret i64)

        ;; allocate `len` bytes and copy the input into them
        (local.set $dst
          (call $realloc (i64.const 0) (i64.const 0) (i64.const 1) (local.get $len)))
        (memory.copy (local.get $dst) (local.get $ptr) (local.get $len))

        ;; allocate the 16-byte `[ptr, len]` return area
        (local.set $ret
          (call $realloc (i64.const 0) (i64.const 0) (i64.const 8) (i64.const 16)))
        (i64.store (local.get $ret) (local.get $dst))
        (i64.store offset=8 (local.get $ret) (local.get $len))
        (local.get $ret))
    )
    (core instance $m (instantiate $m))

    (func (export "roundtrip") (param "a" (list u8)) (result (list u8))
      (canon lift (core func $m "roundtrip")
        (memory $m "memory")
        (realloc (func $m "realloc"))))
  )
  (instance $c64 (instantiate $c64))

  ;; The 32-bit "frontend" component. It imports the 64-bit `roundtrip`,
  ;; lowers it into its own `i32` memory, and re-exports a `roundtrip` that
  ;; forwards straight through.
  (component $c32
    (import "backend" (instance $i
      (export "roundtrip" (func (param "a" (list u8)) (result (list u8))))
    ))

    (core module $libc
      (memory (export "memory") 1)
      (global $next (mut i32) (i32.const 8))
      (func (export "realloc")
        (param $old i32) (param $old_sz i32) (param $align i32) (param $new_sz i32)
        (result i32)
        (local $ret i32)
        (local.set $ret
          (i32.and
            (i32.add (global.get $next) (i32.const 7))
            (i32.const -8)))
        (global.set $next (i32.add (local.get $ret) (local.get $new_sz)))
        (local.get $ret))
    )
    (core instance $libc (instantiate $libc))

    ;; Lowered into `i32` memory: `(param ptr i32) (param len i32) (param retptr i32)`.
    (core func $roundtrip
      (canon lower (func $i "roundtrip")
        (memory $libc "memory")
        (realloc (func $libc "realloc"))))

    (core module $m
      (import "" "memory" (memory 1))
      (import "" "roundtrip" (func $roundtrip (param i32 i32 i32)))

      ;; Lift core function: takes the incoming list `(ptr, len)` and returns a
      ;; pointer to a `[ptr:i32, len:i32]` result structure. We use address 0 as
      ;; the result area (the allocator hands out addresses >= 8) and let the
      ;; lowered call write the resulting list directly into it.
      (func (export "roundtrip") (param $ptr i32) (param $len i32) (result i32)
        (call $roundtrip (local.get $ptr) (local.get $len) (i32.const 0))
        (i32.const 0))
    )
    (core instance $m (instantiate $m
      (with "" (instance
        (export "memory" (memory $libc "memory"))
        (export "roundtrip" (func $roundtrip))
      ))
    ))

    (func (export "roundtrip") (param "a" (list u8)) (result (list u8))
      (canon lift (core func $m "roundtrip")
        (memory $libc "memory")
        (realloc (func $libc "realloc"))))
  )
  (instance $c32 (instantiate $c32 (with "backend" (instance $c64))))

  (export "roundtrip" (func $c32 "roundtrip"))
)

(assert_return
  (invoke "roundtrip" (list.const (u8.const 1) (u8.const 2) (u8.const 3)))
  (list.const (u8.const 1) (u8.const 2) (u8.const 3)))

(assert_return
  (invoke "roundtrip"
    (list.const
      (u8.const 0) (u8.const 255) (u8.const 128) (u8.const 42)
      (u8.const 17) (u8.const 200) (u8.const 3) (u8.const 99)))
  (list.const
    (u8.const 0) (u8.const 255) (u8.const 128) (u8.const 42)
    (u8.const 17) (u8.const 200) (u8.const 3) (u8.const 99)))

(assert_return
  (invoke "roundtrip" (list.const))
  (list.const))
