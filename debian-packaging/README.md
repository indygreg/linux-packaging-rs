# debian-packaging

`debian-packaging` is a library crate implementing functionality related
to Debian packaging. The following functionality is (partially)
implemented:

* Parsing and serializing control files
* Parsing `Release` and `InRelease` files.
* Parsing `Packages` files.
* Fetching Debian repository files from an HTTP server.
* Writing changelog files.
* Reading and writing `.deb` files.
* Creating repositories.
* PGP signing and verification operations.

See the crate's documentation for more.
