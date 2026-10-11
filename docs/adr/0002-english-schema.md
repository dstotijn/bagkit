---
status: superseded by ADR-0008
---

# Use an English schema of our own

bagkit's first schema used Dutch table and column names. With no consumers depending on that format and the goal shifting to a toolkit of BAG primitives, we replaced it with a schema of our own: English names, integer identifiers, and each address linked directly to its addressable object. The schema separates core data, which stores current records as the BAG states them, from derived data that bagkit computes, such as points, WGS84 coordinates and the spatial index. The national database also shrank from 10.7 GiB to 6.6 GiB, an outcome of the change and not its goal.

## Consequences

- Clients and queries written for the Dutch schema do not work against the current one.
- Incompatible schema changes bump `SCHEMA_VERSION` in `src/lib.rs` and are documented on the documentation site's Tables page.
