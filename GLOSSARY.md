# bagkit

bagkit turns the Dutch BAG (Basisregistratie Adressen en Gebouwen) into a queryable snapshot of adressen and panden, and answers address and nearest-address lookups from it.

Terms are the BAG's own, as the extract and Kadaster's BAG API use them. Data (tables, columns, values and the HTTP API) uses them as they are. English prose keeps them too, and uses the English rendering given with a term only where a sentence needs an ordinary English word, such as "address".

## Source data

**BAG**:
The Dutch national register of adressen and gebouwen, kept by gemeenten and distributed by Kadaster.
_Avoid_: Address database, Kadaster data

**Extract**:
A complete delivery of BAG records, such as the national extract PDOK publishes as a ZIP of ZIPs (BAG: extract, LVC-extract).
_Avoid_: Dump, export, download

**Voorkomen**:
One version of a BAG object, valid for a validity period. An object can have many voorkomens over time. In English: record.
_Avoid_: Row, entry, occurrence, version

**Validity period**:
The interval in which a voorkomen describes its object, from its beginGeldigheid up to but excluding its eindGeldigheid; a missing eindGeldigheid means it is still valid.
_Avoid_: Lifetime, active period

**Peildatum**:
The date a conversion selects voorkomens for: the snapshot describes the BAG as it stood on this date. In English: reference date.
_Avoid_: Import date, as-of date

**Current voorkomen**:
A voorkomen whose validity period contains the peildatum and that is neither inactief nor niet-BAG.
_Avoid_: Active record, live record

**Inactief**:
Said of a voorkomen the BAG later replaced with a correction, so it never described its object.
_Avoid_: Deleted record, inactive record, corrected record

**Niet-BAG**:
Said of a voorkomen for an object that was taken out of the BAG.
_Avoid_: Removed record, non-BAG record

## Objects

**Woonplaats**:
A named place, designated by a gemeente, that nummeraanduidingen and openbare ruimten lie in. Identified by its 4-digit woonplaatscode. In English: locality.
_Avoid_: Town, city, place

**Gemeente**:
The local government a woonplaats belongs to. Not part of the BAG: Kadaster supplies each woonplaats's gemeentecode with the extract, and the names come from elsewhere. In English: municipality.
_Avoid_: City, council

**Provincie**:
The region a gemeente belongs to. In English: province.
_Avoid_: State, region

**Openbare ruimte**:
A named street, water, railway, square or other public area that nummeraanduidingen lie on. Its type says which kind it is. In English: public space.
_Avoid_: Street (except for an openbare ruimte of type Weg)

**Nummeraanduiding**:
A postcode and huisnummer, with an optional huisletter and huisnummertoevoeging, on an openbare ruimte. In English: address designation.
_Avoid_: Address (for the BAG object itself), label, location

**Adres**:
A nummeraanduiding with the names of its openbare ruimte and its woonplaats: what someone writes on an envelope. In English: address.
_Avoid_: Location, nummeraanduiding (for the combination)

**Huisnummer**, **huisletter**, **huisnummertoevoeging**:
The number, the single letter after it, and the further addition that together with the postcode identify a nummeraanduiding. In English: house number, house letter, house number addition.
_Avoid_: Number suffix, unit number

**Woonplaats of an adres**:
The woonplaats a nummeraanduiding lies in when it names one, otherwise its openbare ruimte's woonplaats.
_Avoid_: Effective locality, town override

**Ingetrokken nummeraanduiding**:
A nummeraanduiding whose status is Naamgeving ingetrokken. Searches leave it out; a lookup by identificatie returns it. In English: withdrawn.
_Avoid_: Deleted address, retired address

**Adresseerbaar object**:
An object that nummeraanduidingen belong to: a verblijfsobject, a ligplaats or a standplaats. Its type says which. In English: addressable object.
_Avoid_: Dwelling, residence, verblijfsobject (for the general case)

**Verblijfsobject**:
A space inside one or more panden with its own adres, gebruiksdoelen and oppervlakte.
_Avoid_: Dwelling, apartment, unit, residence

**Ligplaats**:
A place in water for a houseboat or other permanently moored vessel.
_Avoid_: Berth, mooring

**Standplaats**:
A place on land for a mobile home or other structure that is not fixed in place.
_Avoid_: Site, lot, pitch

**Hoofdadres**:
The one nummeraanduiding the BAG designates as an adresseerbaar object's main adres. Every adresseerbaar object has exactly one.
_Avoid_: Primary address, main address

**Nevenadres**:
Any other nummeraanduiding of an adresseerbaar object.
_Avoid_: Secondary address, additional address

**Gebruiksdoel**:
What a verblijfsobject may be used for, such as woonfunctie. A verblijfsobject can have several.
_Avoid_: Usage purpose, function, use

**Oppervlakte**:
A verblijfsobject's gebruiksoppervlakte, in whole square metres.
_Avoid_: Area, usable area, floor area

**Pand**:
A permanent structure with its own geometrie. A verblijfsobject can lie in several panden. In English: building.
_Avoid_: Premises, building (in data)

**Oorspronkelijk bouwjaar**:
The year a pand was or will be completed as originally built.
_Avoid_: Construction year, build year

**Identificatie**:
The 16-digit number that identifies a BAG object, or the 4-digit woonplaatscode for a woonplaats. In English: identifier.
_Avoid_: ID (in data), key

## Geometry

**Geometrie**:
The shape the BAG records for an object: a vlak for a pand, ligplaats or standplaats, and a punt or vlak for a verblijfsobject.
_Avoid_: Footprint, shape, outline

**Vlak**:
A polygon geometrie, in RD New.
_Avoid_: Footprint, area

**Punt**:
The single location of an adresseerbaar object: the punt the BAG records, or else the centroid of its vlak.
_Avoid_: Location, position, coordinate

**RD New**:
The Dutch national grid (Rijksdriehoeksstelsel, EPSG:28992), in which the BAG records all geometrie.
_Avoid_: RD, Amersfoort coordinates

**WGS84**:
Breedtegraad and lengtegraad as used by GPS, which bagkit derives from RD New. In English: latitude and longitude.
_Avoid_: GPS coordinates, lat/lon (as names)

## bagkit

**Snapshot**:
An immutable database built from one extract at one peildatum.
_Avoid_: Mirror, replica

**Core data**:
What a snapshot stores as the BAG records it: current voorkomens of each object type and their relations.
_Avoid_: Raw data, base tables

**Derived data**:
What bagkit computes from core data, such as punten, WGS84 coordinates and the spatial index.
_Avoid_: Enriched data, computed columns

**Adreslijst**:
A flat table of current adressen that are not ingetrokken, with their names and punten, made without panden or geometrie. In English: address list.
_Avoid_: Address export, address dump, postcode table

**Address lookup**:
Finding the adressen that match a postcode, optionally narrowed by huisnummer, huisletter and huisnummertoevoeging.
_Avoid_: Search, geocoding

**Nearest lookup**:
Finding the adresseerbaar object with a nummeraanduiding that is not ingetrokken, closest to a point, and returning its adressen.
_Avoid_: Reverse geocoding, nearby search
