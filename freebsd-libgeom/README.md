# freebsd-libgeom

Rusty bindings to FreeBSD's libgeom

[![Build Status](https://api.cirrus-ci.com/github/asomers/gstat-rs.svg)](https://cirrus-ci.com/github/asomers/gstat-rs)
[![Crates.io](https://img.shields.io/crates/v/freebsd-libgeom.svg)](https://crates.io/crates/freebsd-libgeom)
[![Documentation](https://docs.rs/freebsd-libgeom/badge.svg)](https://docs.rs/freebsd-libgeom)

## Overview

libgeom is the userland API Library for the kernel GEOM subsystem.  It's used
to view the GEOM configuration, get I/O statistics for GEOM providers, and send
control requests to GEOM providers.

Currently this library only supports the statistics API.  The other
functionality may be added on an as-needed basis.

# Minimum Supported Rust Version (MSRV)

freebsd-libgeom is supported on Rust 1.52.0 and higher.  It's MSRV will not be
changed in the future without bumping the major or minor version.

# License

`freebsd-libgeom` is primarily distributed under the terms of the BSD 2-clause license.

See LICENSE for details.

# Sponsorship

freebsd-libgeom is sponsored by Axcient, inc.
