//! Keeps the documentation website in step with the code.
//!
//! Reference sections between `<!-- generated: NAME -->` and `<!-- end generated: NAME -->`
//! markers are generated here from the code and from real conversions; the test fails when
//! a page differs, ignoring how the formatter lays out tables. Run `mise run docs:generate`
//! to rewrite and format them. Other checks make sure documented commands parse, and that
//! documented examples have the shape bagkit really produces.
#![cfg(all(feature = "cli", feature = "parquet"))]

mod common;

use bagkit::{
    api,
    cli::Cli,
    convert::{self, METADATA, Options, Shape},
    dataset::{self, ADDRESS_LIST, Format, Kind, Table},
    extract::{Extract, Selection},
    query, tables,
};
use clap::{CommandFactory, Parser};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

fn docs() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web/src/content/docs")
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bag.xml")
}

/// Every page, as path and text.
fn pages() -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, pages: &mut Vec<(PathBuf, String)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, pages);
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("md" | "mdx")
            ) {
                let text = fs::read_to_string(&path).unwrap();
                pages.push((path, text));
            }
        }
    }
    let mut pages = Vec::new();
    walk(&docs(), &mut pages);
    let readme = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("README.md");
    let text = fs::read_to_string(&readme).unwrap();
    pages.push((readme, text));
    pages.sort();
    pages
}

/// Replaces the generated region `name` in `page`, or checks that it is up to date.
/// Returns a complaint when it is not.
fn region(page: &str, name: &str, content: &str) -> Option<String> {
    let path = docs().join(page);
    let text = fs::read_to_string(&path).unwrap();
    let start = format!("<!-- generated: {name} -->\n");
    let end = format!("<!-- end generated: {name} -->");
    let a = text
        .find(&start)
        .unwrap_or_else(|| panic!("{page} has no generated region {name}"))
        + start.len();
    let b = text[a..]
        .find(&end)
        .unwrap_or_else(|| panic!("{page} does not end region {name}"))
        + a;
    let content = format!("{}\n", content.trim_end());
    if normalize(&text[a..b]) == normalize(&content) {
        return None;
    }
    if std::env::var_os("BAGKIT_UPDATE_DOCS").is_some() {
        fs::write(&path, format!("{}{content}{}", &text[..a], &text[b..])).unwrap();
        return None;
    }
    Some(format!(
        "{page}: region {name} is out of date; run `mise run docs:generate`"
    ))
}

/// Text as the formatter may lay it out differently: without blank lines, trailing spaces
/// or the padding of table cells.
fn normalize(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .map(|line| {
            if !line.starts_with('|') {
                return line.to_owned();
            }
            line.trim_matches('|')
                .split('|')
                .map(|cell| {
                    let cell = cell.trim();
                    if !cell.is_empty() && cell.chars().all(|c| c == '-' || c == ':') {
                        "---"
                    } else {
                        cell
                    }
                })
                .collect::<Vec<_>>()
                .join("|")
        })
        .collect()
}

fn table(header: &[&str], rows: &[Vec<String>]) -> String {
    let mut text = format!(
        "| {} |\n|{}\n",
        header.join(" | "),
        " --- |".repeat(header.len())
    );
    for row in rows {
        text.push_str(&format!("| {} |\n", row.join(" | ")));
    }
    text
}

fn kind(kind: Kind, nullable: bool) -> String {
    let name = match kind {
        Kind::Integer => "Integer",
        Kind::Identifier(_) => "Identificatie",
        Kind::Real(_) => "Real",
        Kind::Text => "Text",
        Kind::Boolean => "Boolean",
        Kind::Geometry => "Geometry",
    };
    if nullable {
        format!("{name}, nullable")
    } else {
        name.into()
    }
}

/// A table's columns, with paired columns on one row.
fn columns(table: &Table) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for column in table.columns {
        if column.with_previous
            && let Some(previous) = rows.last_mut()
        {
            previous[0].push_str(&format!(", `{}`", column.name));
            continue;
        }
        rows.push(vec![
            format!("`{}`", column.name),
            kind(column.kind, column.nullable),
            column.description.into(),
        ]);
    }
    table_with_descriptions(&rows)
}

/// Leaves out the description column when no column has one.
fn table_with_descriptions(rows: &[Vec<String>]) -> String {
    if rows.iter().all(|row| row[2].is_empty()) {
        let rows: Vec<Vec<String>> = rows.iter().map(|row| row[..2].to_vec()).collect();
        table(&["Column", "Type"], &rows)
    } else {
        table(&["Column", "Type", "Description"], rows)
    }
}

/// The usage line and options of a command.
fn command(command: &clap::Command, path: &str) -> String {
    let usage = command
        .clone()
        .bin_name(path)
        .render_usage()
        .to_string()
        .replace("Usage: ", "");
    let mut rows = Vec::new();
    for argument in command.get_arguments() {
        let id = argument.get_id().as_str();
        if id == "help" || id == "version" {
            continue;
        }
        let value = argument
            .get_value_names()
            .map(|names| names.join(" "))
            .unwrap_or_else(|| id.to_uppercase());
        let takes_value = argument.get_num_args().is_some_and(|n| n.takes_values());
        let name = if argument.is_positional() {
            let many = if argument.get_num_args().is_some_and(|n| n.max_values() > 1) {
                "..."
            } else {
                ""
            };
            if argument.is_required_set() {
                format!("`<{value}>{many}`")
            } else {
                format!("`[{value}]{many}`")
            }
        } else {
            let mut names = Vec::new();
            if let Some(short) = argument.get_short() {
                names.push(format!("`-{short}`"));
            }
            let long = argument.get_long().unwrap();
            names.push(if takes_value {
                format!("`--{long} <{value}>`")
            } else {
                format!("`--{long}`")
            });
            names.join(", ")
        };
        let defaults: Vec<String> = argument
            .get_default_values()
            .iter()
            .map(|v| v.to_string_lossy().into_owned())
            .collect();
        let default = match defaults.as_slice() {
            [] => String::new(),
            [flag] if !takes_value && flag == "false" => "off".into(),
            values => format!("`{}`", values.join(",")),
        };
        let mut help = argument
            .get_long_help()
            .or(argument.get_help())
            .map(|h| h.to_string())
            .unwrap_or_default()
            .trim_end_matches('.')
            .replace('\n', " ");
        let values: Vec<String> = argument
            .get_possible_values()
            .iter()
            .filter(|_| takes_value)
            .map(|v| format!("`{}`", v.get_name()))
            .collect();
        // Name the possible values unless the help text already explains them.
        let explained = argument
            .get_possible_values()
            .iter()
            .all(|v| help.contains(v.get_name()));
        if !values.is_empty() && !explained {
            help = format!("{help}: {}", values.join(", "));
        }
        rows.push(vec![name, default, help]);
    }
    format!(
        "```txt\n{usage}\n```\n\n{}",
        table(&["Option", "Default", "Description"], &rows)
    )
}

/// The root cause of a conversion of `inputs` that must fail.
fn failure(inputs: &[PathBuf], output: &Path) -> String {
    let extract = Extract::new(inputs.to_vec(), "2026-10-07").unwrap();
    let error = convert::convert(
        &extract,
        &Options {
            output: output.to_owned(),
            force: true,
            format: Format::Sqlite,
            shape: Shape::Tables,
            selection: Selection::all(),
            cache_mib: 8,
        },
    )
    .expect_err("the conversion must fail");
    error.root_cause().to_string()
}

/// One row per import rule, with the message a conversion breaking it really gives.
fn import_rules() -> String {
    let dir = tempfile::tempdir().unwrap();
    let xml = fs::read_to_string(fixture()).unwrap();
    let relations = |localities: &[&str]| {
        let records: String = localities
            .iter()
            .map(|l| {
                format!(
                    "<GemeenteWoonplaatsRelatie><tijdvakgeldigheid><begindatumTijdvakGeldigheid>2000-01-01</begindatumTijdvakGeldigheid></tijdvakgeldigheid><gerelateerdeWoonplaats><identificatie>{l}</identificatie></gerelateerdeWoonplaats><gerelateerdeGemeente><identificatie>0014</identificatie></gerelateerdeGemeente><status>definitief</status></GemeenteWoonplaatsRelatie>"
                )
            })
            .collect();
        format!("<GemeenteWoonplaatsRelatieProduct>{records}</GemeenteWoonplaatsRelatieProduct>")
    };
    let cases: Vec<(&str, Vec<(&str, String)>)> = vec![
        (
            "An identificatie repeats within an object type",
            vec![(
                "bag.xml",
                xml.replace(
                    ">0000200000000002</Objecten:identificatie>",
                    ">0000200000000001</Objecten:identificatie>",
                ),
            )],
        ),
        (
            "An identificatie does not have 16 digits (4 for woonplaatsen)",
            vec![(
                "bag.xml",
                xml.replace(
                    ">0000200000000008</Objecten:identificatie>",
                    ">12</Objecten:identificatie>",
                ),
            )],
        ),
        (
            "A nummeraanduiding belongs to more than one adresseerbaar object",
            vec![("bag.xml", xml.replace(
                "<Objecten-ref:NummeraanduidingRef>0000200000000003</Objecten-ref:NummeraanduidingRef></Objecten:heeftAlsHoofdadres>",
                "<Objecten-ref:NummeraanduidingRef>0000200000000001</Objecten-ref:NummeraanduidingRef></Objecten:heeftAlsHoofdadres>",
            ))],
        ),
        (
            "An adresseerbaar object does not have exactly one hoofdadres",
            vec![("bag.xml", xml.replace(
                "<Objecten:heeftAlsHoofdadres><Objecten-ref:NummeraanduidingRef>0000200000000003</Objecten-ref:NummeraanduidingRef></Objecten:heeftAlsHoofdadres>",
                "<Objecten:heeftAlsNevenadres><Objecten-ref:NummeraanduidingRef>0000200000000003</Objecten-ref:NummeraanduidingRef></Objecten:heeftAlsNevenadres>",
            ))],
        ),
        (
            "An adresseerbaar object refers to a nummeraanduiding that does not exist",
            vec![("bag.xml", xml.replace(
                "<Objecten-ref:NummeraanduidingRef>0000200000000008</Objecten-ref:NummeraanduidingRef>",
                "<Objecten-ref:NummeraanduidingRef>0000200000000077</Objecten-ref:NummeraanduidingRef>",
            ))],
        ),
        (
            "A verblijfsobject refers to a pand that does not exist",
            vec![("bag.xml", xml.replace(
                "<Objecten-ref:PandRef>0000100000000002</Objecten-ref:PandRef>",
                "<Objecten-ref:PandRef>0000100000000009</Objecten-ref:PandRef>",
            ))],
        ),
        (
            "A nummeraanduiding refers to an openbare ruimte that does not exist",
            vec![("bag.xml", xml.replace(
                "<Objecten:huisnummer>112</Objecten:huisnummer><Objecten:postcode>7311KZ</Objecten:postcode><Objecten:status>Naamgeving uitgegeven</Objecten:status><Objecten:ligtAan><Objecten-ref:OpenbareRuimteRef>0000300000000001",
                "<Objecten:huisnummer>112</Objecten:huisnummer><Objecten:postcode>7311KZ</Objecten:postcode><Objecten:status>Naamgeving uitgegeven</Objecten:status><Objecten:ligtAan><Objecten-ref:OpenbareRuimteRef>0000300000000099",
            ))],
        ),
        (
            "A nummeraanduiding refers to a woonplaats that does not exist",
            vec![("bag.xml", xml.replace(
                "<Objecten-ref:WoonplaatsRef>5678</Objecten-ref:WoonplaatsRef></Objecten:ligtIn>",
                "<Objecten-ref:WoonplaatsRef>9999</Objecten-ref:WoonplaatsRef></Objecten:ligtIn>",
            ))],
        ),
        (
            "An openbare ruimte refers to a woonplaats that does not exist",
            vec![("bag.xml", xml.replace(
                "<Objecten-ref:WoonplaatsRef>1234</Objecten-ref:WoonplaatsRef></Objecten:ligtIn>",
                "<Objecten-ref:WoonplaatsRef>9999</Objecten-ref:WoonplaatsRef></Objecten:ligtIn>",
            ))],
        ),
        (
            "A woonplaats has no gemeente, or several, in `GEM-WPL-RELATIE`",
            vec![
                ("GEM-WPL-RELATIE.xml", relations(&["1234"])),
                ("bag.xml", xml.clone()),
            ],
        ),
        (
            "XML, dates or numbers are malformed, or a file contains a DTD",
            vec![("bag.xml", "<!DOCTYPE bag><bag/>".into())],
        ),
    ];
    let rows: Vec<Vec<String>> = cases
        .into_iter()
        .enumerate()
        .map(|(index, (rule, files))| {
            let case = dir.path().join(index.to_string());
            fs::create_dir(&case).unwrap();
            let inputs: Vec<PathBuf> = files
                .iter()
                .map(|(name, text)| {
                    assert!(
                        name != &"bag.xml" || text != &xml || files.len() > 1,
                        "{rule}: the edit must change the fixture"
                    );
                    let path = case.join(name);
                    fs::write(&path, text).unwrap();
                    path
                })
                .collect();
            let message = failure(&inputs, &case.join("out.sqlite"));
            vec![rule.to_owned(), format!("`{message}`")]
        })
        .collect();
    table(&["Rule", "Example message"], &rows)
}

#[test]
fn generated_reference_sections_are_up_to_date() {
    let mut cli = Cli::command();
    // Building fills in what clap derives, such as how many values an option takes.
    cli.build();
    let mut commands = Vec::new();
    for sub in cli.get_subcommands() {
        if sub.get_name() == "help" {
            continue;
        }
        commands.push(vec![
            format!("`{}`", sub.get_name()),
            sub.get_about().map(|a| a.to_string()).unwrap_or_default(),
        ]);
    }
    let mut problems = vec![region(
        "reference/cli.md",
        "cli-commands",
        &table(&["Command", "Description"], &commands),
    )];
    for name in ["convert", "serve", "query"] {
        let sub = cli.find_subcommand(name).unwrap();
        problems.push(region(
            "reference/cli.md",
            &format!("cli-{name}"),
            &command(sub, &format!("bagkit {name}")),
        ));
    }

    let mut sections = String::new();
    for table in tables::tables(Selection::all()) {
        sections.push_str(&format!(
            "## `{}`\n\n{}\n\n{}\n",
            table.name,
            table.description,
            columns(table)
        ));
    }
    problems.push(region("reference/tables.md", "tables", &sections));
    problems.push(region(
        "reference/address-list.md",
        "address-list",
        &columns(&ADDRESS_LIST),
    ));

    let errors: Vec<Vec<String>> = api::ERRORS
        .iter()
        .map(|(status, when)| {
            vec![
                status.to_string(),
                api::title(*status).to_owned(),
                (*when).to_owned(),
            ]
        })
        .collect();
    problems.push(region(
        "reference/http.md",
        "http-errors",
        &table(&["Status", "Title", "When"], &errors),
    ));

    let keys: Vec<Vec<String>> = METADATA
        .iter()
        .map(|(key, description)| vec![format!("`{key}`"), (*description).to_owned()])
        .collect();
    problems.push(region(
        "reference/metadata.md",
        "metadata",
        &table(&["Key", "Description"], &keys),
    ));
    problems.push(region(
        "concepts/import-rules.md",
        "import-rules",
        &import_rules(),
    ));

    let problems: Vec<String> = problems.into_iter().flatten().collect();
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Lines of fenced code blocks in `text` whose language is one of `languages`.
fn code_lines<'a>(text: &'a str, languages: &[&str]) -> Vec<&'a str> {
    let mut lines = Vec::new();
    let mut inside = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(fence) = trimmed.strip_prefix("```") {
            inside = match inside {
                Some(_) => None,
                None => Some(languages.contains(&fence.split_whitespace().next().unwrap_or(""))),
            };
            continue;
        }
        if inside == Some(true) {
            lines.push(trimmed);
        }
    }
    lines
}

#[test]
fn documented_commands_parse() {
    let mut checked = 0;
    for (path, text) in pages() {
        for line in code_lines(&text, &["sh", "bash", "shell"]) {
            let Some(arguments) = line.strip_prefix("bagkit ") else {
                continue;
            };
            // Leave out a trailing shell comment.
            let arguments = arguments.split(" #").next().unwrap_or(arguments);
            let arguments: Vec<&str> = arguments.split_whitespace().collect();
            if let Err(error) = Cli::try_parse_from(std::iter::once("bagkit").chain(arguments)) {
                panic!("{}: `{line}` does not parse:\n{error}", path.display());
            }
            checked += 1;
        }
    }
    assert!(checked > 10, "only {checked} documented commands found");
}

/// The JSON code blocks of a page, one string per block.
fn json_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    // Inside a code block: whether it is JSON.
    let mut inside: Option<bool> = None;
    let mut lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(info) = trimmed.strip_prefix("```") {
            match inside.take() {
                Some(true) => blocks.push(std::mem::take(&mut lines).join("\n")),
                Some(false) => (),
                None => inside = Some(info.split_whitespace().next() == Some("json")),
            }
            continue;
        }
        if inside == Some(true) {
            lines.push(trimmed);
        }
    }
    blocks
}

#[test]
fn documented_api_responses_have_the_real_shape() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("bag.sqlite");
    let extract = Extract::new(vec![fixture()], "2026-10-07").unwrap();
    convert::convert(
        &extract,
        &Options {
            output: db.clone(),
            force: false,
            format: Format::Sqlite,
            shape: Shape::Tables,
            selection: Selection::all(),
            cache_mib: 8,
        },
    )
    .unwrap();
    let database = query::open(&db).unwrap();
    let real = |target: &str| -> BTreeSet<String> {
        let response = api::get(&database, target).unwrap();
        assert_eq!(response.status, 200, "{target}");
        common::shape(&serde_json::from_str(&response.body).unwrap())
    };
    let search =
        real("/v1/adressen?postcode=7311KZ&huisnummer=110&huisletter=A&huisnummertoevoeging=2");
    let nearest = real("/v1/adressen/_dichtstbij?punt=5.962442171959101,52.211733682888124");
    let single = real("/v1/adressen/0000200000000002");
    let root = real("/v1");

    let mut checked = 0;
    for (path, text) in pages() {
        for block in json_blocks(&text) {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&block) else {
                continue;
            };
            let (expected, schema) = if value.pointer("/adressen/0/afstand").is_some() {
                (&nearest, "AdresCollectie")
            } else if value.pointer("/adressen/0/adresseerbaarObject").is_some() {
                (&search, "AdresCollectie")
            } else if value.get("adresseerbaarObject").is_some() {
                (&single, "Adres")
            } else if value.get("dataset").is_some() {
                (&root, "Root")
            } else if value.get("detail").is_some() {
                common::assert_matches_schema("Problem", &value);
                checked += 1;
                continue;
            } else {
                continue;
            };
            common::assert_matches_schema(schema, &value);
            assert_eq!(&common::shape(&value), expected, "{}", path.display());
            checked += 1;
        }
    }
    assert!(
        checked >= 4,
        "only {checked} documented API responses found"
    );
}

#[test]
fn documented_csv_header_and_metadata_match() {
    let header: Vec<&str> = ADDRESS_LIST.column_names().collect();
    let page = fs::read_to_string(docs().join("guides/address-lists.mdx")).unwrap();
    let documented = code_lines(&page, &["csv"]);
    assert_eq!(documented.first().copied(), Some(header.join(",").as_str()));

    let documented_keys: BTreeSet<&str> = METADATA.iter().map(|(key, _)| *key).collect();
    let page = fs::read_to_string(docs().join("reference/metadata.md")).unwrap();
    let example: serde_json::Value =
        serde_json::from_str(&code_lines(&page, &["json"]).join("\n")).unwrap();
    let example_keys: BTreeSet<&str> = example
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(example_keys, documented_keys);

    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("csv");
    let extract = Extract::new(vec![fixture()], "2026-10-07").unwrap();
    convert::convert(
        &extract,
        &Options {
            output: output.clone(),
            force: false,
            format: Format::Csv,
            shape: Shape::Tables,
            selection: Selection::all(),
            cache_mib: 8,
        },
    )
    .unwrap();
    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(output.join("metadata.json")).unwrap()).unwrap();
    for key in written.as_object().unwrap().keys() {
        assert!(
            documented_keys.contains(key.as_str()),
            "{key} is not documented"
        );
    }
    let _ = dataset::Format::Parquet;
}
