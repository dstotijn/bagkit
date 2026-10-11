---
title: Command line
description: Every command and option of the bagkit executable.
---

<!-- generated: cli-commands -->

| Command   | Description                                                                     |
| --------- | ------------------------------------------------------------------------------- |
| `convert` | Convert BAG XML, gzip, ZIPs (including nested national extracts) or directories |
| `serve`   | Serve the HTTP API for address and nearest-address lookups from SQLite output   |
| `query`   | Answer an HTTP API request without running a server                             |

<!-- end generated: cli-commands -->

Every command supports `--help`, and `bagkit --version` prints the version.

## convert

<!-- generated: cli-convert -->

```txt
bagkit convert [OPTIONS] <INPUTS>...
```

| Option                  | Default  | Description                                                                                                                                                                                                                                            |
| ----------------------- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `<INPUTS>...`           |          | XML, .gz or .zip files, or directories searched recursively for them                                                                                                                                                                                   |
| `-o`, `--output <PATH>` |          | The output file, or directory for tables as CSV or Parquet. Defaults to bag.sqlite, bag (a directory) or adreslijst.csv/.parquet                                                                                                                       |
| `--format <FORMAT>`     | `sqlite` | The output format: `sqlite`, `csv`, `parquet`                                                                                                                                                                                                          |
| `--shape <SHAPE>`       | `tables` | tables: one dataset per object type; addresses: the adreslijst, one row per current adres that is not ingetrokken, with its openbare ruimte, woonplaats, gemeente, provincie, type of adresseerbaar object and WGS84 punt                              |
| `--include <TYPES>`     |          | Object types to convert, comma-separated: woonplaatsen, gemeenten, openbare-ruimten, nummeraanduidingen, adresseerbare-objecten, panden, and geometrie (the polygons of panden and adresseerbare objecten). Defaults to all of them. Tables shape only |
| `--exclude <TYPES>`     |          | Object types to leave out, as for --include                                                                                                                                                                                                            |
| `--date <YYYY-MM-DD>`   |          | The peildatum: the date voorkomens must be valid on (defaults to today's local date)                                                                                                                                                                   |
| `--force`               | off      | Atomically replace an existing output after a successful conversion                                                                                                                                                                                    |
| `--cache-mib <MIB>`     | `64`     | SQLite page cache in MiB; sorting uses temporary files                                                                                                                                                                                                 |
| `--threads <N>`         | `0`      | Threads that inflate and parse XML; 0 uses every available core                                                                                                                                                                                        |

<!-- end generated: cli-convert -->

`import` is an alias. Pass either the archives or their extracted contents; passing both reads every voorkomen twice.

When it finishes, `convert` prints how many voorkomens it read and converted, the output, for an adreslijst the number of adressen, and how long the conversion took in total and per phase: load and publish, and for SQLite also index. Any warnings follow, such as objects whose coordinates lie outside RD New.

## serve

<!-- generated: cli-serve -->

```txt
bagkit serve [OPTIONS] [PORT]
```

| Option                                | Default      | Description                                                               |
| ------------------------------------- | ------------ | ------------------------------------------------------------------------- |
| `[PORT]`                              | `8080`       | The port to listen on                                                     |
| `--database <PATH>`                   | `bag.sqlite` | A SQLite output                                                           |
| `--host <HOST>`                       | `0.0.0.0`    | The address to bind                                                       |
| `--workers <N>`                       | `4`          | Database workers answering queries                                        |
| `--max-connections <N>`               | `256`        | Maximum accepted connections; excess sockets are closed immediately       |
| `--connection-timeout-secs <SECONDS>` | `30`         | Total connection lifetime, including headers, queries and response writes |

<!-- end generated: cli-serve -->

See the [HTTP API](/reference/http/).

## query

<!-- generated: cli-query -->

```txt
bagkit query [OPTIONS] <TARGET>
```

| Option              | Default      | Description                                                                     |
| ------------------- | ------------ | ------------------------------------------------------------------------------- |
| `--database <PATH>` | `bag.sqlite` | A SQLite output                                                                 |
| `<TARGET>`          |              | A request path and query, for example /v1/adressen?postcode=9902HA&huisnummer=1 |

<!-- end generated: cli-query -->

Prints the result as JSON, the same as the HTTP API returns it.
