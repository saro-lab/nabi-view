# block 0.1.6 compatibility patch

Reviewed: 2026-11-07.

This is the published `block 0.1.6` source used by `metal 0.31.0` and
`wgpu-hal 24.0.4`, selected with the root `[patch.crates-io]` entry.

- Package: https://crates.io/crates/block/0.1.6
- Upstream: https://github.com/SSheldon/rust-block
- Original crate archive SHA-256 (Cargo.lock):
  `0d8c1fef690941d3e7788d328517591fecc684c084084702d6ff1641e993699a`
- Original `src/lib.rs` SHA-256:
  `eb31678adf63b53109d9b94eba23699fd5f9ebfdb950f6e1a57ad51bb6a146fa`
- Author and declared license: Steven Sheldon, MIT. The published package does
  not include a license file; the existing notice generator continues to flag
  this for upstream attribution review. The original README and author/license
  metadata are retained.

The only runtime change replaces the empty `Class` enum with an inhabited,
opaque `#[repr(C)]` struct. `_NSConcreteStackBlock` is only addressed, never
read or instantiated in Rust. The block layout, symbol name, pointer, public
API, copy and release operations are unchanged. The legacy implicit C ABI is
also written explicitly as `extern "C"` throughout to address `missing_abi`
without changing the calling convention.

The manifest explicitly retains Rust 2015, disables publishing this local copy,
and removes the nonexistent `test_utils` path from the upstream test dependency
while retaining its registry version. Upstream test sources remain unchanged.

References:

- https://doc.rust-lang.org/rustc/lints/listing/warn-by-default.html#uninhabited-static
- https://clang.llvm.org/docs/Block-ABI-Apple.html
- https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html#the-patch-section

Remove this patch when the graphics dependencies no longer resolve `block 0.1.6`
or a compatible upstream release fixes the declaration. Do not edit the global
Cargo registry cache or disable future-incompatibility reporting.
