# Make bagkit a library with selectable inputs, shapes and output formats

bagkit is a Rust crate first: reading an extract, shaping its records and writing them are separate library layers, and the `bagkit` binary only parses arguments and calls them. Reading streams the current records of the object types a caller selects, skipping archives nothing needs. A shape turns records into rows: `tables`, one dataset per object type as the SQLite core schema has them, or `addresses`, the flat address list. A writer stores rows as SQLite, CSV or Parquet. We chose this over adding flags to the SQLite importer because each new output would otherwise duplicate the parallel pipeline and the import rules, and because library users need the records without a database.

## Consequences

- Reading happens in two parts: first everything addresses depend on (localities, municipality relations, public spaces, addressable objects), then addresses and buildings. Addresses therefore always get their object from memory, in any input order, and every format can stream rows instead of updating them afterwards.
- Import rules (duplicate identifiers, address links, references) are checked in memory, the same way for every format. A reference is checked only when both sides are read; excluding an object type means its references stay as the BAG states them, unverified, and link tables to it are not written.
- Only SQLite gets indexes, the address point index, the `address_details` view and GeoPackage registration, so `query` and `serve` need SQLite output. CSV and Parquet hold the core and derived columns.
- Geometry is encoded per format: GeoPackage in SQLite, WKB in GeoParquet, WKT in CSV.
- Optional parts are crate features (`sqlite`, `parquet`, `server`, `cli`), so library users do not compile an HTTP server or Parquet unless they ask for them.
