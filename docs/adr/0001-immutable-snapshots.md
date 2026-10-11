# Build immutable snapshots for one reference date

An import selects the current records for a single reference date and writes them to a new database, which then replaces the old one atomically. bagkit does not store history: keeping every record would cost space and complexity that an address service does not need. Applying Kadaster's mutation deliveries to an existing database is deferred rather than rejected; integer primary keys leave room for it.

## Consequences

- Updating the data means a full import, which needs disk space for the old and new database side by side.
- Questions about earlier dates need another import at that reference date.
- Running servers keep reading the old database until they restart.
