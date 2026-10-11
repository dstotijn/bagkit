# Bulk load the address point index by writing R*Tree nodes directly

SQLite has no bulk load for R*Tree tables, and inserting 10 million points one at a time took about 30 seconds on the national extract. `src/rtree.rs` instead sorts points along a Hilbert curve, packs them into full nodes, and writes those nodes into the R*Tree's shadow tables in the format SQLite's `rtree.c` uses. This takes about 5 seconds.

## Consequences

- The importer depends on SQLite's internal R*Tree node format, which SQLite does not document as a stable interface. Tests validate the result with `rtreecheck()`; run them after upgrading SQLite.
- The index cannot be updated after import, which fits immutable snapshots (ADR-0001).
