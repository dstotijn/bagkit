---
title: Metadata
description: What every output records about how it was made.
---

SQLite outputs record metadata in a `bagkit_metadata` table of keys and values. CSV and Parquet tables put the same keys in `metadata.json` beside the table files.

```json title="bag/metadata.json"
{
  "schema_version": "1",
  "layout": "tables",
  "reference_date": "2026-10-08",
  "version": "0.1.0",
  "municipality_names": "PDOK brk-bestuurlijke-gebieden gemeentegebied, retrieved 2026-10-10",
  "object_types": "woonplaatsen,gemeenten,openbare-ruimten,nummeraanduidingen,adresseerbare-objecten,panden",
  "geometry": "included",
  "extract_date": "2026-10-08"
}
```

<!-- generated: metadata -->

| Key                  | Description                                                                 |
| -------------------- | --------------------------------------------------------------------------- |
| `schema_version`     | The layout version. `query` and `serve` refuse databases of another version |
| `layout`             | `tables` or `address_list` (an adreslijst)                                  |
| `reference_date`     | The peildatum voorkomens were selected for                                  |
| `version`            | The bagkit version that wrote the output                                    |
| `municipality_names` | Where bagkit's municipality table came from, and when                       |
| `object_types`       | The object types that were read                                             |
| `geometry`           | `included` or `omitted`: whether geometrie was read                         |
| `extract_date`       | The extract's `StandTechnischeDatum`, when the inputs declare one date      |

<!-- end generated: metadata -->

In SQLite:

```sql
SELECT key, value FROM bagkit_metadata;
```

A single-file CSV or Parquet address list carries no metadata.
