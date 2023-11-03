# `debian-packaging` History

<!-- next-header -->

## Unreleased

Released on ReleaseDate.

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
