# Change Log

All notable changes to this project will be documented in this file.
This project adheres to [Semantic Versioning](http://semver.org/).

## [ 0.2.0 ] - 2021-07-30

### Fixed

- Fix `Gident::{name, rank}` methods.  Previously they would return
  uninitialized data when called on GEOM consumers.  Now they will correctly
  check for the fields validity.  This changes the signature of the
  `Gident::name` method.
  (#[7](https://github.com/asomers/gstat-rs/pull/7))
