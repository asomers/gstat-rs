// vim: tw=80
//! Rust FFI bindings for FreeBSD's libgeom library
//!
//! These are raw, `unsafe` FFI bindings.  Here be dragons!  You probably
//! shouldn't use this crate directly.  Instead, you should use the
//! [`freebsd-libgeom`](https://crates.io/crates/freebsd-libgeom) crate.
#![cfg_attr(crossdocs, doc = "")]
#![cfg_attr(crossdocs, doc = "These docs are just stubs!  Don't trust them.")]
// bindgen generates some unconventional type names
#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]

#[cfg(not(crossdocs))]
#[cfg(target_pointer_width = "32")]
mod ffi32;
#[cfg(not(crossdocs))]
#[cfg(target_pointer_width = "64")]
mod ffi64;
#[cfg(not(crossdocs))]
#[cfg(target_pointer_width = "32")]
pub use ffi32::*;
#[cfg(not(crossdocs))]
#[cfg(target_pointer_width = "64")]
pub use ffi64::*;

#[cfg(crossdocs)]
mod fakes;
#[cfg(crossdocs)]
pub use fakes::*;
