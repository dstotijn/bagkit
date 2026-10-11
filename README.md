# bagkit

[![CI](https://github.com/dstotijn/bagkit/actions/workflows/ci.yml/badge.svg)](https://github.com/dstotijn/bagkit/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**A toolkit for the Dutch register of addresses and buildings (BAG). Convert to SQLite, CSV and Parquet. Address lookup via HTTP.**

bagkit reads the monthly national extract from Kadaster straight from its ZIP file and keeps
only what is current on the date you choose. The output uses the BAG's own names for tables
and columns, and works directly in GIS tools: SQLite output is a GeoPackage, Parquet output is
GeoParquet. Once you have a SQLite database, bagkit can look up _adressen_ (addresses) by
postcode or find the nearest one to a coordinate, from the command line or through a Dutch HTTP
API that follows the government's API Design Rules. It comes as both a command-line tool and a
[Rust library](https://dstotijn.github.io/bagkit/guides/library/).

**Documentation: [dstotijn.github.io/bagkit](https://dstotijn.github.io/bagkit/)**

## Quick start

Install bagkit (macOS and Linux; [other options](#install)):

```sh
curl -fsSL https://dstotijn.github.io/bagkit/install.sh | sh
```

Download the national extract (about 3.6 GB) and convert it to `bag.sqlite`:

```sh
curl -O https://service.pdok.nl/kadaster/adressen/atom/v1_0/downloads/lvbag-extract-nl.zip
bagkit convert lvbag-extract-nl.zip
```

Look up an _adres_ by postcode and _huisnummer_ (house number), or the nearest _adres_ to a point:

```sh
bagkit query '/v1/adressen?postcode=9902HA&huisnummer=1'
bagkit query '/v1/adressen/_dichtstbij?punt=6.86877,53.31934'
```

Serve the same requests over HTTP, described in OpenAPI at `/v1/openapi.json`:

```sh
bagkit serve 8080
```

Or write other formats and shapes:

```sh
bagkit convert lvbag-extract-nl.zip --format parquet --output bag   # GeoParquet, one file per table
bagkit convert lvbag-extract-nl.zip --shape addresses --format csv  # one row per address
bagkit convert lvbag-extract-nl.zip --exclude panden,geometrie      # skip the largest part
```

## Install

The install script downloads the latest [release](https://github.com/dstotijn/bagkit/releases)
for your platform, verifies its SHA-256 checksum and installs it to `~/.local/bin`. To pin a
version or install elsewhere, set variables for `sh`:

```sh
curl -fsSL https://dstotijn.github.io/bagkit/install.sh | BAGKIT_VERSION=v0.1.0 BAGKIT_INSTALL_DIR=/opt/bin sh
```

To install by hand, download `bagkit-<target>.tar.gz` (or `.zip` on Windows) and its `.sha256`
file from the releases page, check it with `shasum -a 256 -c bagkit-<target>.tar.gz.sha256`,
unpack it and put `bagkit` on your `PATH`. To build from source, run
`cargo install --locked bagkit` (Rust 1.88 or later).

Release binaries are available for macOS and Linux, on arm64 and x86_64, and for Windows on
x86_64. The Linux binaries are statically linked. bagkit has SQLite compiled in and no runtime
dependencies. See [Installation](https://dstotijn.github.io/bagkit/installation/) for the
memory and disk a national conversion needs.

## How it works

```text
lvbag-extract-nl.zip ──▶ read ─────────────▶ shape ─────────────────▶ write ──▶ SQLite, Parquet, CSV
                         nested ZIPs,        rows per table or         one atomic
                         parallel parsing,   adreslijst, import        publish
                         current voorkomens  rules checked
```

- **Read: stream the extract.** One thread copies each XML document out of its ZIP still
  compressed, nested ZIPs included, so nothing is unpacked to disk. Worker threads, one per core
  by default, inflate, parse and convert documents, and never run more than a few documents ahead
  of the writer, so memory stays flat. _Voorkomens_ (records) whose validity ended before the
  _peildatum_ (reference date), and documents of object types nobody selected, are skipped
  without being parsed.
- **Two parts.** _Nummeraanduidingen_ (address designations) refer to _adresseerbare objecten_
  (addressable objects), _openbare ruimten_ (public spaces) and _woonplaatsen_ (localities), so
  those are read first and kept in memory. _Nummeraanduidingen_ and _panden_ (buildings) follow
  in the second part, and every format writes each row once, as it streams.
- **Shape: tables or an _adreslijst_ (address list).** The `tables` shape writes one dataset
  per object type, linked by _identificaties_ (identifiers). The `addresses` shape writes one
  flat row per current _adres_, with its _openbare ruimte_, _woonplaats_, _gemeente_
  (municipality), _provincie_ (province) and coordinates already joined. Along the way bagkit
  checks _identificaties_ and references, and fails an import with a clear message rather than
  store inconsistent data.
- **Write: snapshots, not updates.** Each monthly extract becomes a fresh output rather than
  changes applied to an old one. The output is built beside the destination and replaces it only
  once it is complete, so readers never see a half-written database. SQLite output gets its
  indexes and a bulk-loaded spatial index once loading is done.
- **Look up: SQLite and an R-tree.** `bagkit query` and `bagkit serve` answer the same
  requests. Searches use the postcode index; nearest lookups search the spatial index outward
  from the point, converting WGS84 to RD New as the EPSG dataset defines it.

See [How bagkit works](https://dstotijn.github.io/bagkit/concepts/architecture/) for the full
picture, and [`docs/adr/`](docs/adr) for the decisions behind it.

## License

[MIT](LICENSE). bagkit is not affiliated with Kadaster or PDOK. The BAG data is published by
Kadaster under [CC0](https://creativecommons.org/publicdomain/zero/1.0/).

---

© 2026 David Stotijn
