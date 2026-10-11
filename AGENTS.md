# Repository Guidelines

## Purpose

`bagkit` is a Rust toolkit for the Dutch BAG (building and address data): a library that reads BAG extracts, shapes their current voorkomens into tables or an adreslijst, and writes them as SQLite, CSV or Parquet, plus a `bagkit` command that uses it and serves a Dutch HTTP API for address and nearest-address lookups from SQLite. See ADR-0006 for the layering.

Outputs have two layers, described on the documentation site's Tables page: a core that stores current voorkomens as the BAG records them, and derived data that bagkit computes. Keep that split and bump `SCHEMA_VERSION` in `src/lib.rs` for incompatible changes. New behaviour belongs in the library; the CLI only maps arguments onto it. Preserve correctness when optimizing, and prefer failing an import with a clear message over storing inconsistent data.

## Project Structure & Module Organization

Reading: `src/source.rs` finds the XML documents in inputs, `src/xml.rs` scans BAG records and converts selected ones into typed objects, and `src/extract.rs` runs the parallel two-part read over selected object types. Shaping: `src/dataset.rs` defines tables, values and per-format encoding, `src/tables.rs` and `src/address_list.rs` turn records into rows and check import rules in memory (`src/ids.rs`, `src/municipalities.rs`, `src/derived.rs` help). Writing: `src/output.rs` publishes outputs and writes CSV, `src/sqlite.rs` and `src/parquet.rs` write their formats, and `src/convert.rs` ties the layers together. `src/geometry.rs` and `src/geo.rs` handle polygons and coordinates, `src/rtree.rs` bulk loads the spatial index, `src/query.rs` implements lookups, `src/api.rs` shapes them into the HTTP API that `src/openapi.json` describes, `src/server.rs` serves it, and `src/cli.rs` serves the `bagkit` command. SQL for indexes, the derived layer and GeoPackage registration lives beside these modules. Optional parts are crate features (`sqlite`, `parquet`, `server`, `cli`); check that `cargo build --lib --no-default-features` still builds.

The public documentation website lives in `web/` (see `web/AGENTS.md`). Integration tests and small synthetic fixtures live in `tests/`. Reproducible import and lookup benchmarks live in `scripts/` and `benches/`; their generated datasets and databases belong in ignored `tmp/`, separate from fixtures. Read the documentation pages for an area before changing its behavior.

## Build, Test, and Development Commands

Use the pinned toolchain with `mise exec --` before these commands:

- `cargo build --release`: build the optimized executable.
- `cargo test`: run unit, integration, and documentation tests.
- `cargo fmt --check`: check formatting.
- `cargo clippy --all-targets -- -D warnings`: check lint warnings.
- `cargo deny check`: check dependencies for advisories and licenses (`deny.toml`).

`mise run check` runs all of these. `mise run docs:check` also builds the documentation site and checks its links and accessibility. Run `mise run benchmark --count 300000 --repeats 3` for synthetic import measurements. CLI examples are in `README.md`.

## Coding Style & Naming Conventions

Use `rustfmt` defaults, including four-space indentation. Use `snake_case` for modules and functions, `PascalCase` for types, and `SCREAMING_SNAKE_CASE` for constants. Name things by the Language rules below. Keep changes focused and justify unsafe code with explicit safety invariants.

## Language

Data speaks the BAG's Dutch; bagkit itself speaks English (ADR-0008). `GLOSSARY.md` holds every term; add a new one there before using it.

- **BAG terms, as the BAG writes them:** table and column names, values, and the whole HTTP API: resources, fields, parameters, problem details and the OpenAPI description. Take names from the extract and from Kadaster's BAG API Individuele Bevragingen, in snake case for tables and camel case for the API.
- **English:** Rust identifiers, command line options and shape names, metadata keys, commit messages, and the documentation site.
- **Comments relate code to the BAG:** a doc comment on a type, variant or field that stands for a BAG concept names its BAG term, such as `/// A nummeraanduiding.` on `Address`, and other comments name BAG concepts by their glossary term.
- **English sentences with BAG nouns:** log and error messages and documentation prose name BAG concepts by their glossary term, such as "nummeraanduidingen refer to missing woonplaatsen". Use a term's English rendering only where a sentence needs an ordinary English word.

## Testing Guidelines

Use Rust's built-in test framework; no coverage threshold is established. Name tests after observable behavior. Cover validity periods, inactief and niet-BAG voorkomens, nevenadressen, verblijfsobjecten in several panden, huisletter casing, woonplaats overrides, polygon dimensions and holes, and coordinate conversion. Check behavior against the national extract when data assumptions change.

Don't state timings, such as durations, latencies or throughput, in code, comments, documentation, ADRs, commit messages or summaries of your work: they depend on the machine that measured them. Describe what a change does to the work instead, such as fewer passes over the data, one query instead of many, or bounded memory, and point to the benchmark commands so readers can measure on their own hardware. The only place for timings is a pull request's evidence for a performance change: measure release builds on reproducible datasets, before and after on the same machine, and report the hardware, dataset size, elapsed time and peak memory. Favor bounded memory and streaming where appropriate; verify optimizations with measurements and regression tests.

## Documentation

The site in `web/src/content/docs` is the user documentation; the README only summarises it and links to it. Keep the documentation in the same commit as the behaviour it describes:

- **Find the pages.** `scripts/docs_map.toml` maps code to the pages that describe it. CI fails a pull request that changes mapped code without touching one of its pages, unless it carries the `docs-not-needed` label.
- **Regenerate references.** The command line options, tables and columns, adreslijst, metadata keys, HTTP errors and import rule messages are generated from the code into marked regions (`<!-- generated: ... -->`). Change the source (clap help, `Column::describe`, `convert::METADATA`, `api::ERRORS`) rather than the page, then run `mise run docs:generate`. `cargo test` fails when a page is out of date.
- **Keep examples real.** Rust examples live in `examples/` and the library guide imports them; `cargo test` compiles them. `tests/docs.rs` checks that every documented `bagkit` command parses, that documented API responses match the OpenAPI description and the real shape, that the adreslijst header is real, and that documented metadata keys exist. Take other example output from real runs, and state no timings (see Testing Guidelines).
- **Check the site.** `mise run docs:format` formats it with oxfmt; `mise run docs:check` checks formatting, lints with oxlint, type-checks, builds without warnings, checks links and accessibility, and runs the drift test.
- **Use one source.** Explain behaviour on one page and link to it from others, rather than repeating it.

## Commit & Pull Request Guidelines

CI (`.github/workflows/ci.yml`) also tests on macOS and Windows, checks the `rust-version` in `Cargo.toml`, dry-runs `cargo publish` and lints the workflows. Pushing a `v*` tag that matches the `Cargo.toml` version builds release binaries, creates a GitHub release and publishes the crate to crates.io (`release.yml`). `pages.yml` deploys the documentation site to GitHub Pages from `main`.

Use concise, imperative subjects, such as `Add streaming BAG parser`. Pull requests should explain the purpose, link relevant issues, report validation, and include benchmark evidence for performance claims. Call out schema and HTTP changes explicitly; an HTTP change also updates `src/openapi.json`, `api::RESOURCES` for parameters, `api::VERSION` and the changelog on the HTTP API page, and passes `mise run api:check`; `tests/openapi.rs` and `tests/http.rs` fail when the description and the code disagree.

## Agent skills

### Issue tracker

Issues live in GitHub Issues on `dstotijn/bagkit`, managed with the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Uses the default label names: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `GLOSSARY.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.
