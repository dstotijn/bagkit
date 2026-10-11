# Fail imports on inconsistent source data

The schema relies on assumptions about the BAG: integer primary keys require unique identifiers, and storing each address's addressable object on the address is only correct if no address belongs to two objects. An import therefore stops, naming an example, when the extract breaks them: duplicate or malformed identifiers, an address claimed by two objects, an object without exactly one main address, or a reference to a missing record. Loading such data anyway would make lookups silently return wrong or duplicated results.

The national extract showed why this matters: its corrected and non-BAG records reuse the identifiers of current records, which before they were skipped produced 247,596 duplicate identifiers.

## Consequences

- An extract with a genuine data error cannot be imported until the importer learns to handle that case explicitly.
- Lookups can assume one result per address and one main address per object.
