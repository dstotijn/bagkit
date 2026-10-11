---
title: The BAG
description: What the Basisregistratie Adressen en Gebouwen holds, and the terms bagkit uses for it.
---

The BAG (Basisregistratie Adressen en Gebouwen) is the Dutch national register of addresses and buildings. Gemeenten maintain it, and Kadaster distributes it. Every month PDOK publishes the full register as one **extract**: a ZIP of ZIPs holding about 105 GiB of XML. Kadaster's [download page](https://www.kadaster.nl/-/gratis-download-bag-extract) links to the latest one; a new extract appears on the 8th of each month.

bagkit uses the BAG's own terms, as the extract and Kadaster's [BAG API](https://lvbag.github.io/BAG-API/) use them: in table and column names, in the HTTP API, and in this documentation. The [glossary](https://github.com/dstotijn/bagkit/blob/main/GLOSSARY.md) defines each one.

## Object types

| BAG              | In English   | What it is                                                                                             |
| ---------------- | ------------ | ------------------------------------------------------------------------------------------------------ |
| Woonplaats       | Locality     | A named place, designated by a gemeente                                                                |
| Openbare ruimte  | Public space | A named street, water, square or other public area that nummeraanduidingen lie on                      |
| Nummeraanduiding | Address      | A postcode and huisnummer, with an optional huisletter and huisnummertoevoeging, on an openbare ruimte |
| Verblijfsobject  | Unit         | A space inside one or more panden with its own adres, gebruiksdoelen and oppervlakte                   |
| Ligplaats        | Mooring      | A place in water for a houseboat or other permanently moored vessel                                    |
| Standplaats      | Pitch        | A place on land for a mobile home or other structure that is not fixed in place                        |
| Pand             | Building     | A permanent structure with its own geometrie                                                           |

Verblijfsobjecten, ligplaatsen and standplaatsen together are **adresseerbare objecten**: the things nummeraanduidingen belong to.

## How they relate

- A nummeraanduiding lies on one openbare ruimte, and an openbare ruimte lies in one woonplaats.
- A nummeraanduiding can name its own woonplaats, which overrides its openbare ruimte's. A nummeraanduiding with the names of its openbare ruimte and woonplaats is an **adres**.
- An adresseerbaar object has exactly one **hoofdadres** and any number of **nevenadressen**. The BAG records this on the object; bagkit stores it on the [nummeraanduiding](/reference/tables/#nummeraanduidingen): which object it belongs to, and whether it is a nevenadres.
- A verblijfsobject lies in one or more panden, and a pand can hold many verblijfsobjecten.

## Gemeenten and provincies

Gemeenten are not part of the BAG. Kadaster ships a separate file with the extract, `GEM-WPL-RELATIE`, that relates each woonplaats to a gemeentecode. bagkit reads it, and names gemeenten and their provincies from a table built into bagkit, generated from PDOK's administrative boundaries. After a municipal reorganisation, new codes have no name until that table is regenerated.

## Voorkomens and history

The extract holds every version of every object. Each version is a **voorkomen** with a validity period. bagkit keeps only the voorkomens that are current on a [peildatum](/concepts/reference-dates/).

Two kinds of voorkomens never count as current:

- **Inactief**: versions the BAG later replaced with a correction, so they never described their object.
- **Niet-BAG**: objects that were taken out of the BAG.

## Statuses

Every object has a status, such as `Verblijfsobject in gebruik` or `Naamgeving ingetrokken`. bagkit keeps statuses and other code-list values as the BAG records them. A nummeraanduiding with status `Naamgeving ingetrokken` is **ingetrokken**: it stays in the `nummeraanduidingen` table, but searches and the adreslijst leave it out.

## Coordinates

The BAG records geometrie in **RD New** (Rijksdriehoeksstelsel, EPSG:28992), the Dutch national grid in metres. bagkit keeps geometrie in RD New and adds a WGS84 breedtegraad and lengtegraad for every adresseerbaar object's punt.
