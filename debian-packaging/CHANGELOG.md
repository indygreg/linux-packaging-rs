# `debian-packaging` History

<!-- next-header -->

## Unreleased

Released on ReleaseDate.

* MSRV 1.70 -> 1.75.
* `async-std` 1.12 -> 1.13.
* `async-tar` 0.4 -> 0.5.
* `bytes` 1.5 -> 1.8.
* `libflate` 2.0 -> 2.1.
* `mailparse` 0.14 -> 0.15.
* `once_cell` 1.18 -> 1.20.
* `os_str_bytes` 6.6 -> 7.0.
* `pgp` 0.10 -> 0.14.
* `regex` 1.10 -> 1.11.
* `reqwest` 0.11 -> 0.12.
* `smallvec` 1.11 -> 1.13.
* `strum` 0.25 -> 0.26.
* `strum_macros` 0.25 -> 0.26.
* `tokio` 1.33 -> 1.41.
* `url` 2.4 -> 2.5.
* `tempfile` 3.8 -> 3.13.

## 0.17.0

Released on 2023-11-03.

* MSRV 1.62 -> 1.70.
* Package version lexical comparison reworked to avoid sorting.
* `.deb` tar archives now correctly encode directories as directory entries.
* Release files with `Contents*` files in the top-level directory are now
  parsed without error. The stored `component` field is now an
  `Option<T>` and various APIs taking a `component: &str` now take
  `Option<&str>` since components are now optional.
* Various package updates to latest versions.

## 0.16.0 and Earlier

* No changelog kept.
