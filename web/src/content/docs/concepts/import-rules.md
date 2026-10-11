---
title: Import rules
description: The checks every conversion makes, and what happens when data breaks them.
---

bagkit prefers failing a conversion with a clear message over writing inconsistent data. Every format gets the same checks, and a failed conversion leaves any previous output untouched.

## What is checked

A conversion fails, naming an example, when:

<!-- generated: import-rules -->

| Rule                                                                     | Example message                                                                                         |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------- |
| An identificatie repeats within an object type                           | `duplicate Nummeraanduiding identificatie 0000200000000001`                                             |
| An identificatie does not have 16 digits (4 for woonplaatsen)            | `identificatie '12' is not 16 digits`                                                                   |
| A nummeraanduiding belongs to more than one adresseerbaar object         | `1 nummeraanduidingen belong to more than one adresseerbaar object, for example 0000200000000001`       |
| An adresseerbaar object does not have exactly one hoofdadres             | `Standplaats 0000030000000001 has 0 hoofdadressen`                                                      |
| An adresseerbaar object refers to a nummeraanduiding that does not exist | `1 links from adresseerbare objecten refer to missing nummeraanduidingen, for example 0000200000000077` |
| A verblijfsobject refers to a pand that does not exist                   | `1 links from verblijfsobjecten refer to missing panden, for example 0000100000000009`                  |
| A nummeraanduiding refers to an openbare ruimte that does not exist      | `nummeraanduidingen refer to missing openbare ruimten, for example 0000300000000099`                    |
| A nummeraanduiding refers to a woonplaats that does not exist            | `nummeraanduidingen refer to missing woonplaatsen, for example 9999`                                    |
| An openbare ruimte refers to a woonplaats that does not exist            | `1 openbare ruimten refer to missing woonplaatsen, for example 9999`                                    |
| A woonplaats has no gemeente, or several, in `GEM-WPL-RELATIE`           | `1 woonplaatsen have no gemeente in GEM-WPL-RELATIE, for example 5678`                                  |
| XML, dates or numbers are malformed, or a file contains a DTD            | `DTD declarations are not supported in BAG extracts`                                                    |

<!-- end generated: import-rules -->

Only current voorkomens are checked. A historical voorkomen that is malformed or incomplete does not stop a conversion, since bagkit never uses it.

## When both sides are read

A reference is checked only when both object types were read. If you [exclude](/guides/converting/#choosing-object-types) openbare ruimten, nummeraanduidingen keep their `openbare_ruimte_identificatie` as the BAG records it, unverified. The [metadata](/reference/metadata/) records which types were read, so you can tell.

## Why

bagkit's tables use identificaties as primary keys and store each nummeraanduiding's adresseerbaar object on the nummeraanduiding. Both are only correct when identificaties are unique and every nummeraanduiding has at most one object. Checking up front means lookups can rely on one row per nummeraanduiding and one hoofdadres per object. See [ADR-0003](https://github.com/dstotijn/bagkit/blob/main/docs/adr/0003-fail-imports-on-inconsistent-data.md).
