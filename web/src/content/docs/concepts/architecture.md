---
title: How bagkit works
description: The read, shape and write layers, and how they divide the work.
---

bagkit is a library in three layers, and the `bagkit` command ties them together:

1. **Read** (`extract`): find the XML documents in the inputs, parse them in parallel, and keep the voorkomens of selected object types that are current on the peildatum.
2. **Shape** (`tables`, `address_list`): turn voorkomens into rows, check the [import rules](/concepts/import-rules/), and encode values for the output format.
3. **Write** (`output`, `sqlite`, `parquet`): store the rows and publish the output in one step.

The design and its trade-offs are recorded in [ADR-0006](https://github.com/dstotijn/bagkit/blob/main/docs/adr/0006-library-with-pluggable-outputs.md).

## The pipeline

One thread reads the inputs and copies each XML document out of its ZIP still compressed. Nested ZIPs are streamed: their entries are cut out of the inflating stream, so nothing is unpacked to disk. Worker threads, one per core by default, inflate, parse and convert documents into encoded rows. The calling thread writes the rows in document order, so each table receives its voorkomens in identificatie order, and the workers never run more than a few documents ahead of it, which keeps memory flat.

## Two parts

Nummeraanduidingen need what they refer to: their adresseerbaar object, their openbare ruimte, their woonplaats. Reading happens in two parts so those are in memory first:

1. **First part:** woonplaatsen, the relations between woonplaatsen and gemeenten, openbare ruimten and adresseerbare objecten, plus any document whose name names no object type.
2. **Second part:** nummeraanduidingen and panden.

The extract names its documents by type (`WPL`, `GEM-WPL-RELATIE`, `OPR`, `VBO`, `LIG`, `STA`, `NUM`, `PND`), so bagkit assigns documents to parts by name and skips the ones nobody selected without reading them. This works for any input order, and lets every format write each row once, as it streams.

## Parsing fast

- Each XML document is inflated into a buffer the worker reuses and scanned in place, looking only for markup and checking that end tags match. Larger documents and plain files are streamed through [quick-xml](https://github.com/tafia/quick-xml) instead.
- The scanner looks ahead to each voorkomen's end tag and skips voorkomens whose validity ended on or before the peildatum without parsing them: about half of the extract.
- Identificaties kept for the checks are delta-encoded while they arrive in order, about two bytes each.

## Writing fast

- **SQLite:** many rows per statement, sorted by key, so each table appends. Indexes and the spatial index are built once loading is done; the spatial index is bulk-loaded in Hilbert order.
- **Parquet:** each row group of 65,536 rows is encoded and compressed on a helper thread while the next one fills.
- **CSV:** workers format rows, so the writer only copies bytes.

`mise run benchmark` and `cargo bench --bench lookup` measure conversions and lookups on your own machine. None of this needs unsafe Rust: bagkit's own code has none.
