# Change Log

All notable changes to this project will be documented in this file.
This project adheres to [Semantic Versioning](http://semver.org/).

## [ Unreleased ] - ReleaseDate

### Fixed

- Don't panic when pressing the arrow keys on a screwed-up terminal.
  ([4218445](https://github.com/asomers/gstat-rs/commit/4218445d63cc864d315bbd5ece15a75457213822))

- When deleting a column with Del, persist that change to the config file.
  ([f406beb](https://github.com/asomers/gstat-rs/commit/f406beb5c8ad6160ded471e2658af22aedb5552d))

- Only use side-by-side mode if it can be done without truncating names.
  (#[8](https://github.com/asomers/gstat-rs/pull/8))

- Don't crash at startup if the config file is corrupt, if `--reset-config` is
  used.
  (#[6](https://github.com/asomers/gstat-rs/pull/6))

- Don't display GEOM consumers.  By mistake, gstat sometimes displayed GEOM
  consumers.  Their names were blank.
  (#[7](https://github.com/asomers/gstat-rs/pull/7))

- Removed the useless `columns` option from the help menu.
  (#[5](https://github.com/asomers/gstat-rs/pull/5))

## [ 0.1.1 ] - 2021-07-16

### Fixed

- Always have a column selected in the column selector
- Fix crashes with up and down arrows for empty tables
- Fix crashes with really small terminals
- When using up and down, pass through deselected before wrapping around
- Accept the 'q' key to quit directly from the column selector menu.
- Don't use the alternate screen.
- Fixed display initial stats at startup.
