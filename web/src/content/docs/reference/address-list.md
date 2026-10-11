---
title: Address list
description: The columns of the adreslijst, which the addresses shape writes.
---

One row per adres that is current on the peildatum and not ingetrokken. See [Address lists](/guides/address-lists/) for how to make one. In SQLite the table is called `adreslijst`, and its columns have the names of the `adressen` view in a database with tables.

<!-- generated: address-list -->

| Column                           | Type           | Description                                                                         |
| -------------------------------- | -------------- | ----------------------------------------------------------------------------------- |
| `nummeraanduiding_identificatie` | Identificatie  |                                                                                     |
| `openbare_ruimte_naam`           | Text           |                                                                                     |
| `huisnummer`                     | Integer        |                                                                                     |
| `huisletter`                     | Text, nullable |                                                                                     |
| `huisnummertoevoeging`           | Text, nullable |                                                                                     |
| `postcode`                       | Text, nullable |                                                                                     |
| `woonplaats_naam`                | Text           | The effective woonplaats: the nummeraanduiding's own, or else its openbare ruimte's |
| `gemeente_naam`                  | Text, nullable | The woonplaats's gemeente                                                           |
| `provincie_naam`                 | Text, nullable | The gemeente's provincie                                                            |
| `type_adresseerbaar_object`      | Text, nullable | `Verblijfsobject`, `Ligplaats` or `Standplaats`; empty without an object            |
| `breedtegraad`, `lengtegraad`    | Real, nullable | The object's point in WGS84; nevenadressen share it                                 |
| `openbare_ruimte_identificatie`  | Identificatie  |                                                                                     |

<!-- end generated: address-list -->

Types are written as for [tables](/reference/tables/#column-types). Rows appear in identificatie order for the national extract.

A SQLite adreslijst has an index on `postcode` and `huisnummer`, so `query` and `serve` answer address lookups from it. A CSV or Parquet adreslijst is a single file and carries no metadata.
