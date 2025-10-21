# Change Log

All notable changes to this project will be documented in this file.
This project adheres to [Semantic Versioning](http://semver.org/).

## [Unreleased] - ReleaseDate

### Added

- Added `RUST_LOG` support.
  (#[51](https://github.com/asomers/gstat-rs/pull/51))

### Fixed

- Handle `ECONNABORTED` errors without exiting.  This completely removes
  dependencies on the unmaintained `prometheus-exporter` and `tiny-http`
  crates.
  (#[55](https://github.com/asomers/gstat-rs/pull/55))

- Better error handling for invalid command line options.
  (#[52](https://github.com/asomers/gstat-rs/pull/52))

## [0.1.1] - 2024-04-18

### Fixed

- Added missing license file
  (#[40](https://github.com/asomers/gstat-rs/pull/40))
