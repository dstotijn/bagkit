---
title: Tables
description: Every table and column of the tables shape, and how each format writes them.
---

The tables shape writes the current voorkomens as the BAG records them, with the BAG's names, plus a few columns bagkit derives. In SQLite they are tables of one database; in CSV and Parquet they are one file each, named after the table.

## Column types

| Type          | SQLite           | Parquet        | CSV                                                         |
| ------------- | ---------------- | -------------- | ----------------------------------------------------------- |
| Identificatie | `INTEGER`        | `INT64`        | Zero-padded to 16 digits, 4 for woonplaatsen and gemeenten  |
| Integer       | `INTEGER`        | `INT64`        | Decimal                                                     |
| Real          | `REAL`           | `DOUBLE`       | RD New to 3 decimals (millimetres), WGS84 to 7 (about 1 cm) |
| Text          | `TEXT`           | UTF-8 string   | Quoted only when it contains a comma, quote or line break   |
| Boolean       | `INTEGER` 0 or 1 | `BOOLEAN`      | `true` or `false`                                           |
| Geometry      | GeoPackage blob  | GeoParquet WKB | WKT                                                         |

Missing values are `NULL` in SQLite and Parquet and empty in CSV. In SQLite, `printf('%016d', id)` gives an identifier's official form, and text comparisons such as `id = '0363010000000001'` still use the primary key.

<!-- generated: tables -->

## `woonplaatsen`

Woonplaatsen.

| Column          | Type                    | Description                                                                          |
| --------------- | ----------------------- | ------------------------------------------------------------------------------------ |
| `identificatie` | Identificatie           | The woonplaatscode                                                                   |
| `naam`          | Text                    |                                                                                      |
| `status`        | Text                    | `Woonplaats aangewezen` or `Woonplaats ingetrokken`                                  |
| `geconstateerd` | Boolean                 |                                                                                      |
| `gemeente_code` | Identificatie, nullable | From `GEM-WPL-RELATIE`, which Kadaster supplies alongside the BAG; `NULL` without it |

## `gemeenten`

Names and provinces from bagkit's built-in table, for the gemeenten in `GEM-WPL-RELATIE`.

| Column           | Type           | Description                            |
| ---------------- | -------------- | -------------------------------------- |
| `code`           | Identificatie  | The gemeentecode                       |
| `naam`           | Text, nullable | `NULL` for a code bagkit's table lacks |
| `provincie_naam` | Text, nullable |                                        |

## `openbare_ruimten`

Openbare ruimten.

| Column                     | Type           | Description                                                  |
| -------------------------- | -------------- | ------------------------------------------------------------ |
| `identificatie`            | Identificatie  |                                                              |
| `naam`                     | Text           |                                                              |
| `verkorte_naam`            | Text, nullable | The NEN 5825 short name, for names longer than 24 characters |
| `type`                     | Text           | `Weg`, `Water`, and so on                                    |
| `status`                   | Text           | `Naamgeving uitgegeven` or `Naamgeving ingetrokken`          |
| `woonplaats_identificatie` | Identificatie  | Ligt in                                                      |

## `nummeraanduidingen`

Nummeraanduidingen, including withdrawn ones.

| Column                               | Type                    | Description                                                                                                                  |
| ------------------------------------ | ----------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| `identificatie`                      | Identificatie           |                                                                                                                              |
| `postcode`                           | Text, nullable          |                                                                                                                              |
| `huisnummer`                         | Integer                 |                                                                                                                              |
| `huisletter`                         | Text, nullable          | Compared case-insensitively in SQLite                                                                                        |
| `huisnummertoevoeging`               | Text, nullable          | Compared case-insensitively in SQLite                                                                                        |
| `status`                             | Text                    | `Naamgeving uitgegeven` or `Naamgeving ingetrokken`                                                                          |
| `openbare_ruimte_identificatie`      | Identificatie           | Ligt aan                                                                                                                     |
| `woonplaats_identificatie`           | Identificatie, nullable | Ligt in; `NULL` means the openbare ruimte's woonplaats                                                                       |
| `adresseerbaar_object_identificatie` | Identificatie, nullable | The object that has this nummeraanduiding as its hoofdadres or a nevenadres; `NULL` when none, or when objects were not read |
| `indicatie_nevenadres`               | Boolean, nullable       | Whether it is a nevenadres of that object rather than its hoofdadres                                                         |

## `adresseerbare_objecten`

Verblijfsobjecten, ligplaatsen and standplaatsen.

| Column                        | Type               | Description                                                                             |
| ----------------------------- | ------------------ | --------------------------------------------------------------------------------------- |
| `identificatie`               | Identificatie      |                                                                                         |
| `type`                        | Text               | `Verblijfsobject`, `Ligplaats` or `Standplaats`                                         |
| `status`                      | Text               |                                                                                         |
| `gebruiksdoelen`              | Text               | A sorted JSON array, such as `["woonfunctie"]`; empty for ligplaatsen and standplaatsen |
| `oppervlakte`                 | Integer, nullable  | Gebruiksoppervlakte in square metres, for verblijfsobjecten                             |
| `vlak`                        | Geometry, nullable | The object's polygon, when it has one and geometry was read                             |
| `rd_x`, `rd_y`                | Real, nullable     | Derived: the object's punt, or its vlak's area-weighted centroid, in RD New             |
| `breedtegraad`, `lengtegraad` | Real, nullable     | Derived: the point in WGS84; `NULL` outside RD New's area of use                        |

## `panden`

Panden.

| Column                    | Type               | Description                       |
| ------------------------- | ------------------ | --------------------------------- |
| `identificatie`           | Identificatie      |                                   |
| `oorspronkelijk_bouwjaar` | Integer            |                                   |
| `status`                  | Text               |                                   |
| `geometrie`               | Geometry, nullable | `NULL` when geometry was not read |

## `verblijfsobject_panden`

Which panden each verblijfsobject maakt deel uit van. Written only when both adresseerbare objecten and panden are read.

| Column                          | Type          |
| ------------------------------- | ------------- |
| `verblijfsobject_identificatie` | Identificatie |
| `pand_identificatie`            | Identificatie |

<!-- end generated: tables -->

## SQLite extras

A SQLite output also has:

- **`adressen`**, a view with one row per nummeraanduiding, including ingetrokken ones: the names of its openbare ruimte, woonplaats, gemeente and provincie, and its adresseerbaar object and punt. Its column names follow Kadaster's BAG API, such as `openbare_ruimte_naam` and `nummeraanduiding_status`.
- **`adresseerbare_objecten_rtree`**, an integer R*Tree in RD New millimetres over adresseerbare objecten with at least one current nummeraanduiding, used by nearest lookups.
- **GeoPackage registration** of `panden` and `adresseerbare_objecten`, when geometrie was read.
- **`bagkit_metadata`**; see [Metadata](/reference/metadata/).
