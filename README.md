# Rust utilities involving FreeBSD libgeom

[![Build Status](https://api.cirrus-ci.com/github/asomers/gstat-rs.svg)](https://cirrus-ci.com/github/asomers/gstat-rs)

## Overview

This repository contains bindings for libgeom(3) and multiple utilities that
use it.  The original and most important is gstat.  In total, they are:

* gstat: like /usr/sbin/gstat, but better with large numbers of disks. 
[![Crates.io](https://img.shields.io/crates/v/gstat.svg)](https://crates.io/crates/gstat)

* freebsd-geom-exporter: export geom statistics to Prometheus. [![Crates.io](https://img.shields.io/crates/v/freebsd-geom-exporter.svg)](https://crates.io/crates/freebsd-geom-exporter)

* freebsd-libgeom: idiomatic Rust bindings to libgeom(3). [![Crates.io](https://img.shields.io/crates/v/freebsd-libgeom.svg)](https://crates.io/crates/freebsd-libgeom)

* freebsd-libgeom-sys: low-level bindings.  Don't use these directly. [![Crates.io](https://img.shields.io/crates/v/freebsd-libgeom-sys.svg)](https://crates.io/crates/freebsd-libgeom-sys)
