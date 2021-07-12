# gstat-rs

An enhanced replacement for FreeBSD's gstat(8) utility.

[![Build Status](https://api.cirrus-ci.com/github/asomers/gstat-rs.svg)](https://cirrus-ci.com/github/asomers/gstat-rs)
[![Crates.io](https://img.shields.io/crates/v/gstat.svg)](https://crates.io/crates/gstat)

## Overview

`gstat` is awesome, but it has some limitations that come into play on larger
systems.  `gstat-rs` is designed to work better even on servers with hundreds of
disks.  The key differences are:

* gstat-rs does not support batch mode (`-b`) output, but hopefully will in the
  future.
* gstat-rs does not display GEOM consumers (`-c`), but it can easily be
  added if there's any demand for that feature.

# Minimum Supported Rust Version (MSRV)

gstat-rs is supported on Rust 1.52.0 and higher.  It's MSRV will not be
changed in the future without bumping the major or minor version.

# License

`gstat-rs` is primarily distributed under the terms of the BSD 2-clause license.

See LICENSE for details.

# Sponsorship

gstat-rs is sponsored by Axcient, inc.
