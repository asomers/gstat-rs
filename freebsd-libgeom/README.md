# freebsd-libgeom

Rusty bindings to FreeBSD's libgeom

[![Build Status](https://img.shields.io/github/actions/workflow/status/asomers/gstat-rs/ci.yml?branch=master&style=flat-square)](https://github.com/asomers/gstat-rs/actions/workflows/ci.yml?query=branch%3Amaster)
[![Crates.io](https://img.shields.io/crates/v/freebsd-libgeom.svg)](https://crates.io/crates/freebsd-libgeom)
[![Documentation](https://docs.rs/freebsd-libgeom/badge.svg)](https://docs.rs/freebsd-libgeom)

## Overview

libgeom is the userland API Library for the kernel GEOM subsystem.  It's used
to view the GEOM configuration, get I/O statistics for GEOM providers, and send
control requests to GEOM providers.

Currently this library only supports the statistics API.  The other
functionality may be added on an as-needed basis.

# Minimum Supported Rust Version (MSRV)

freebsd-libgeom does not guarantee any specific MSRV.  Rather, it guarantees
compatibility with the oldest rustc shipped in the current FreeBSD ports tree.

* https://www.freshports.org/lang/rust/

# License

`freebsd-libgeom` is primarily distributed under the terms of the BSD 2-clause license.

See LICENSE for details.

# Sponsorship

freebsd-libgeom is sponsored by Axcient, inc.
