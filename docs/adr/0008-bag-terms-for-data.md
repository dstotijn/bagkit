# Name data with the BAG's own Dutch terms

ADR-0002 gave bagkit an English schema. Its users mostly work with the BAG in Dutch, Kadaster's documentation and BAG API name everything in Dutch, and the NLGov API Design Rules ask for Dutch interfaces unless an official English glossary exists, which the BAG has not. English renderings also blurred distinctions the BAG makes: "address" stood for both the nummeraanduiding and the adres, and "unit", "mooring" and "pitch" hid the type names the BAG and its users search for. We switched every name that describes data to the BAG's terms: tables (`nummeraanduidingen`, `adresseerbare_objecten`, `panden`), columns (`huisnummertoevoeging`, `oorspronkelijk_bouwjaar`), values (`Verblijfsobject`), the HTTP API (`/adressen`, `exacteMatch`) and its messages. Where Kadaster's BAG API names a concept, we use its name, such as `indicatie_nevenadres` and `openbare_ruimte_naam`; snake case in tables, camel case in the API.

What describes bagkit itself stays English: Rust identifiers, command line options and shape names, metadata keys, log and error sentences (which name BAG objects in Dutch), and the documentation site. The glossary records each term with the English word prose may use for it.

## Consequences

- Every table and column is renamed, and the schema version restarts at 1, since nothing had been published. Databases and queries made before the switch do not carry over; convert the extract again.
- Rust types such as `Address` and `Building` keep English names. Their doc comments name the BAG term they stand for, and comments elsewhere use the BAG terms, so the code reads against the glossary without renaming it.
- English prose mixes in Dutch nouns, such as "a nummeraanduiding's openbare ruimte". The glossary keeps their use consistent.
