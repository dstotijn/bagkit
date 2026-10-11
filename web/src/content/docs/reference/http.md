---
title: HTTP API
description: Resources, parameters, responses and limits of the API bagkit serve offers.
---

```sh
bagkit serve 8080 --database bag.sqlite
curl 'http://localhost:8080/v1/adressen?postcode=9902HA&huisnummer=1'
```

The API lives under `/v1` and follows the Dutch government's [API Design Rules](https://gitdocumentatie.logius.nl/publicatie/api/adr/) (NLGov ADR 2.2) and their [Geospatial module](https://gitdocumentatie.logius.nl/publicatie/api/mod-geo/). As those rules ask, it is in Dutch: resources, fields, parameters and messages use the BAG's terms, named as in Kadaster's [BAG API Individuele Bevragingen](https://lvbag.github.io/BAG-API/) where it has them. The server describes itself in an OpenAPI 3.0 document at `/v1/openapi.json`, which tools such as Swagger UI and client generators read; it is the authoritative contract, and this page explains it.

## Resources

All resources answer `GET` and `HEAD`, and return JSON.

| Resource                                       | Returns                                                                         |
| ---------------------------------------------- | ------------------------------------------------------------------------------- |
| `/v1`                                          | The API version, the snapshot the server reads, and a link to the description   |
| `/v1/openapi.json`                             | The OpenAPI description                                                         |
| `/v1/crss`                                     | The supported coordinate reference systems                                      |
| `/v1/adressen`                                 | Adressen with a postcode, narrowed by huisnummer, huisletter and toevoeging     |
| `/v1/adressen/{nummeraanduidingIdentificatie}` | One adres by its nummeraanduiding's identificatie, including an ingetrokken one |
| `/v1/adressen/_dichtstbij`                     | The adressen of the adresseerbaar object nearest to a point                     |

Paths have no trailing slash, except the root `/v1/`, which answers like `/v1`. Anything else is a 404.

The root says which snapshot the server reads: its [peildatum](/concepts/reference-dates/), the extract's date when known, and whether it is a database with tables or an adreslijst.

```json
{
  "titel": "bagkit",
  "beschrijving": "Adressen uit een momentopname van de Basisregistratie Adressen en Gebouwen (BAG).",
  "versie": "1.0.0",
  "dataset": {
    "indeling": "tabellen",
    "peildatum": "2026-10-08",
    "extractdatum": "2026-10-08"
  },
  "links": [
    { "rel": "self", "href": "/v1", "type": "application/json" },
    {
      "rel": "service-desc",
      "href": "/v1/openapi.json",
      "type": "application/vnd.oai.openapi+json;version=3.0"
    }
  ]
}
```

## Finding adressen

`/v1/adressen` needs a `postcode` and returns its current adressen, leaving out ingetrokken nummeraanduidingen, ordered by huisnummer, huisletter, huisnummertoevoeging and identificatie.

| Parameter              | Meaning                                                                                  |
| ---------------------- | ---------------------------------------------------------------------------------------- |
| `postcode`             | Required. Four digits, the first not 0, and two letters, without a space: `9902HA`       |
| `huisnummer`           | A huisnummer from 1 to 99999                                                             |
| `huisletter`           | One letter, matched case-insensitively                                                   |
| `huisnummertoevoeging` | One to four letters or digits, matched case-insensitively                                |
| `exacteMatch`          | `true` makes a left-out huisletter or huisnummertoevoeging mean none; needs `huisnummer` |
| `crs`                  | The CRS of `geometrie` in the response; see [Coordinates](#coordinates)                  |

Without `exacteMatch`, `postcode=7311KZ&huisnummer=110` also finds 110A and 110-2. With `exacteMatch=true` it finds only the adres without a huisletter or huisnummertoevoeging, so a full adres names one result at most.

```json
{
  "adressen": [
    {
      "nummeraanduidingIdentificatie": "0003200000137381",
      "postcode": "9902HA",
      "huisnummer": 1,
      "huisletter": null,
      "huisnummertoevoeging": null,
      "openbareRuimteNaam": "Abel Eppensstraat",
      "openbareRuimteIdentificatie": "0003300000116985",
      "woonplaatsNaam": "Appingedam",
      "woonplaatsIdentificatie": "3386",
      "gemeenteNaam": "Eemsdelta",
      "provincieNaam": "Groningen",
      "status": "Naamgeving uitgegeven",
      "indicatieNevenadres": false,
      "adresseerbaarObject": {
        "identificatie": "0003010000129210",
        "type": "Verblijfsobject",
        "status": "Verblijfsobject in gebruik",
        "gebruiksdoelen": ["woonfunctie"],
        "oppervlakte": 118,
        "geometrie": { "type": "Point", "coordinates": [6.86877201, 53.31933815] },
        "panden": [
          {
            "identificatie": "0003100000119616",
            "oorspronkelijkBouwjaar": 1960,
            "status": "Pand in gebruik"
          }
        ]
      }
    }
  ]
}
```

`null` means the BAG has no value: no huisletter, or no adresseerbaar object for the nummeraanduiding. `indicatieNevenadres` says whether the adres is a nevenadres of its object rather than its hoofdadres. `oppervlakte` is in square metres, `gebruiksdoelen` are in alphabetical order, and `geometrie` is the object's punt as a GeoJSON Point. Statuses and gebruiksdoelen are as the BAG records them.

A request for one adres, such as `/v1/adressen/0003200000137381`, returns it in the same form, without the `adressen` wrapper. It also finds an ingetrokken nummeraanduiding, whose `status` is `Naamgeving ingetrokken`.

## Nearest lookup

`/v1/adressen/_dichtstbij` returns the current adressen of the adresseerbaar object nearest to a point, hoofdadres first, each with its `afstand` in metres:

```sh
curl 'http://localhost:8080/v1/adressen/_dichtstbij?punt=6.86877,53.31934'
```

| Parameter | Meaning                                                                                        |
| --------- | ---------------------------------------------------------------------------------------------- |
| `punt`    | Required. Two numbers separated by a comma: lengtegraad and breedtegraad, or x and y in RD New |
| `puntCrs` | The CRS of `punt`; CRS84 when left out                                                         |
| `crs`     | The CRS of `geometrie` in the response                                                         |

The search widens from 100 m up to 25.6 km. Ligplaatsen and standplaatsen count as well as verblijfsobjecten, ties go to the smallest identificatie, and a point farther from every adres, or outside the Netherlands, returns an empty list. [Looking up addresses](/guides/lookups/#nearest-lookup) explains how the search works.

## Coordinates

Punten are GeoJSON Points in one of two coordinate reference systems, named by URI as the Geospatial module asks:

| CRS                    | URI                                            | Coordinates                                 |
| ---------------------- | ---------------------------------------------- | ------------------------------------------- |
| WGS84 (CRS84), default | `http://www.opengis.net/def/crs/OGC/1.3/CRS84` | Lengtegraad and breedtegraad, to 8 decimals |
| RD New (EPSG:28992)    | `http://www.opengis.net/def/crs/EPSG/0/28992`  | x and y in metres, to 3 decimals            |

Pass the URI as `crs` for the response and as `puntCrs` for the point you search from, percent-encoded or as it is. Responses with geometrie name their CRS in a `Content-Crs` header:

```http
Content-Crs: <http://www.opengis.net/def/crs/OGC/1.3/CRS84>
```

A database with tables stores punten in RD New; an adreslijst stores WGS84, which `/v1/crss` reports as `storageCrs`. Outside RD New's area of use an object has no WGS84 punt, so its `geometrie` in CRS84 is `null`.

## From an adreslijst

An [adreslijst](/guides/address-lists/) stores less than a database with tables, so the API leaves out what it does not have rather than returning `null`: `woonplaatsIdentificatie`, `status`, `indicatieNevenadres`, and every field of `adresseerbaarObject` except `type` and `geometrie`. `/v1` reports the `indeling` as `adreslijst`. Nearest lookups need the spatial index of a database with tables and answer 501.

## Headers

Every response carries:

| Header                                                         | Value                                                     |
| -------------------------------------------------------------- | --------------------------------------------------------- |
| `API-Version`                                                  | The full API version, such as `1.0.0`                     |
| `Access-Control-Allow-Origin`                                  | `*`: any web page may read the API                        |
| `Access-Control-Expose-Headers`                                | `API-Version, Content-Crs`                                |
| `Cache-Control`                                                | `no-store`                                                |
| `Content-Security-Policy`                                      | `default-src 'none'; frame-ancestors 'none'`              |
| `Strict-Transport-Security`                                    | `max-age=31536000`, which browsers honour only over HTTPS |
| `X-Content-Type-Options`, `X-Frame-Options`, `Referrer-Policy` | `nosniff`, `DENY`, `no-referrer`                          |

An `Accept` header must allow `application/json`, for example `application/json`, `application/*` or `*/*`; otherwise the answer is 406.

## Errors

Errors are [problem details](https://www.rfc-editor.org/rfc/rfc9457) with the type `application/problem+json`, with Dutch titles and details. A 400 lists every rejected parameter at once in `invalidParams`, each with a `code`: `required`, `invalid`, `repeated`, `unknown` (not a parameter of the resource) or `conflict`.

```json
{
  "status": 400,
  "title": "Ongeldige aanvraag",
  "detail": "ongeldige parameters: postcode, houseNumber",
  "invalidParams": [
    {
      "name": "postcode",
      "code": "invalid",
      "reason": "moet bestaan uit vier cijfers, waarvan het eerste geen 0 is, en twee letters, zoals 7311KZ"
    },
    { "name": "houseNumber", "code": "unknown", "reason": "is geen parameter van deze resource" }
  ]
}
```

<!-- generated: http-errors -->

| Status | Title                   | When                                                                                     |
| ------ | ----------------------- | ---------------------------------------------------------------------------------------- |
| 400    | Ongeldige aanvraag      | A parameter is missing, unknown, repeated or invalid; `invalidParams` lists each problem |
| 404    | Niet gevonden           | The path is not a resource, or no nummeraanduiding has the identification                |
| 405    | Methode niet toegestaan | Anything but `GET` or `HEAD`; `Allow` lists both                                         |
| 406    | Niet aanvaardbaar       | `Accept` rules out `application/json`                                                    |
| 500    | Interne serverfout      | The query failed; the server logs why                                                    |
| 501    | Niet geïmplementeerd    | A nearest lookup against an address list, which has no spatial index                     |
| 503    | Dienst niet beschikbaar | The database workers are not available, for example while the server stops               |

<!-- end generated: http-errors -->

## Versions

The `v1` in the path is the major version. Changes that keep existing clients working, such as a new resource or field, raise the minor or patch version in `API-Version` and stay under `/v1`; an incompatible change starts `/v2`.

| Version | Changes                                                                                                                                  |
| ------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| 1.0.1   | The OpenAPI description documents the `Access-Control-Allow-Origin` header on `/openapi.json`, which the API already sent.               |
| 1.0.0   | The first versioned API, in Dutch. It replaces the unversioned paths such as `/9902HA/1` and `/53.31934/6.86877`, which no longer exist. |

## Deviations from the API Design Rules

bagkit follows the rules except where a self-hosted tool cannot:

- **Documentation in English.** The OpenAPI description is in Dutch, as the rules ask, but this documentation site is in English, which the rules allow for existing documentation.
- **No TLS.** `bagkit serve` speaks plain HTTP. The rules require TLS for every exchange, so put it behind a reverse proxy that terminates TLS before exposing it. For the same reason the description's server URL is the relative `/v1`, not an `https` address, which the checker warns about.
- **Contact information.** The description names bagkit's maintainer and issue tracker as its contact. An organisation that runs bagkit for others should name its own contact.

`mise run api:check` checks the description with the [DON Checker](https://developer.overheid.nl/kennisbank/api-ontwikkeling/tools/api-design-rules-linter) of developer.overheid.nl.

## Limits

- A fixed pool of `--workers` database connections answers queries, behind a queue of twice that size. When the queue is full, requests wait for room within their connection's lifetime.
- At most `--max-connections` connections (256) are accepted; more are closed immediately.
- Each connection lives at most `--connection-timeout-secs` (30), including reading headers, waiting in the queue, querying, idle keep-alive and writing the response. This can cut off a slow reader mid-response.
- Request headers are limited to 64 fields and 16 KiB.
- Keep-alive works within the connection lifetime.

A server keeps reading the database it opened. After [replacing](/guides/updating/) it, restart the server.
