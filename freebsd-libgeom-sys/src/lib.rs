// vim: tw=80
//! Rust FFI bindings for FreeBSD's libgeom library
//!
//! These are raw, `unsafe` FFI bindings.  Here be dragons!  You probably
//! shouldn't use this crate directly.  Instead, you should use the
//! [`freebsd-libgeom`](https://crates.io/crates/freebsd-libgeom) crate.

// bindgen generates some unconventional type names
#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]

// bindgen generates UB in unit tests
// https://github.com/rust-lang/rust-bindgen/issues/1651
#![cfg_attr(test, allow(deref_nullptr))]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
