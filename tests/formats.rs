//! Every format holds the same rows.
#![cfg(all(feature = "sqlite", feature = "parquet"))]

use bagkit::{
    convert::{self, Options, Shape},
    dataset::{Format, Kind, Table},
    extract::{Extract, ObjectType, Selection},
    tables,
};
use parquet::{
    file::{reader::FileReader, serialized_reader::SerializedFileReader},
    record::Field,
};
use rusqlite::{Connection, types::Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bag.xml")
}

fn convert(output: &Path, format: Format, shape: Shape, selection: Selection) {
    let extract = Extract::new(vec![fixture()], "2026-10-07").unwrap();
    convert::convert(
        &extract,
        &Options {
            output: output.to_owned(),
            force: false,
            format,
            shape,
            selection,
            cache_mib: 8,
        },
    )
    .unwrap();
}

/// A value as text, the way CSV writes it; geometry as the WKB inside GeoPackage blobs, in
/// hex.
fn canonical(kind: Kind, value: Option<String>, wkb: Option<Vec<u8>>) -> String {
    match (kind, wkb) {
        (Kind::Geometry, Some(wkb)) => wkb.iter().map(|b| format!("{b:02x}")).collect(),
        _ => value.unwrap_or_default(),
    }
}

fn sqlite_rows(path: &Path, table: &Table) -> Vec<Vec<String>> {
    let db = Connection::open(path).unwrap();
    let mut statement = db
        .prepare(&format!("SELECT * FROM {} ORDER BY 1, 2", table.name))
        .unwrap();
    statement
        .query_map([], |row| {
            Ok(table
                .columns
                .iter()
                .enumerate()
                .map(|(i, column)| {
                    let value = row.get::<_, Value>(i).unwrap();
                    let text = match (column.kind, &value) {
                        (_, Value::Null) => None,
                        (Kind::Identifier(digits), Value::Integer(id)) => {
                            Some(format!("{id:0digits$}"))
                        }
                        (Kind::Boolean, Value::Integer(b)) => Some((*b != 0).to_string()),
                        (Kind::Real(decimals), Value::Real(r)) => Some(format!("{r:.decimals$}")),
                        (_, Value::Integer(i)) => Some(i.to_string()),
                        (_, Value::Text(t)) => Some(t.clone()),
                        _ => None,
                    };
                    // GeoPackage blobs hold an 8-byte header and a 32-byte envelope.
                    let wkb = match value {
                        Value::Blob(blob) => Some(blob[40..].to_vec()),
                        _ => None,
                    };
                    canonical(column.kind, text, wkb)
                })
                .collect())
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn parquet_rows(path: &Path, table: &Table) -> Vec<Vec<String>> {
    let reader = SerializedFileReader::new(fs::File::open(path).unwrap()).unwrap();
    let mut rows: Vec<Vec<String>> = reader
        .get_row_iter(None)
        .unwrap()
        .map(|row| {
            let row = row.unwrap();
            table
                .columns
                .iter()
                .zip(row.get_column_iter())
                .map(|(column, (name, field))| {
                    assert_eq!(name, column.name);
                    let text = match (column.kind, field) {
                        (_, Field::Null) => None,
                        (Kind::Identifier(digits), Field::Long(id)) => {
                            Some(format!("{id:0digits$}"))
                        }
                        (_, Field::Long(i)) => Some(i.to_string()),
                        (_, Field::Bool(b)) => Some(b.to_string()),
                        (Kind::Real(decimals), Field::Double(r)) => Some(format!("{r:.decimals$}")),
                        (_, Field::Str(s)) => Some(s.clone()),
                        _ => None,
                    };
                    let wkb = match field {
                        Field::Bytes(bytes) => Some(bytes.data().to_vec()),
                        _ => None,
                    };
                    canonical(column.kind, text, wkb)
                })
                .collect()
        })
        .collect();
    rows.sort();
    rows
}

/// CSV rows; WKT footprints are compared through SQLite's WKB instead, so they are
/// dropped here and in `expected`.
fn csv_rows(path: &Path) -> Vec<Vec<String>> {
    let text = fs::read_to_string(path).unwrap();
    let mut rows = Vec::new();
    let (mut row, mut field, mut quoted, mut chars) =
        (Vec::new(), String::new(), false, text.chars().peekable());
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            ('"', _) => quoted = !quoted,
            (',', false) => row.push(std::mem::take(&mut field)),
            ('\n', false) => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            (c, _) => field.push(c),
        }
    }
    rows.remove(0);
    rows.sort();
    rows
}

fn without_geometry(table: &Table, rows: &[Vec<String>]) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            row.iter()
                .zip(table.columns)
                .filter(|(_, c)| c.kind != Kind::Geometry)
                .map(|(v, _)| v.clone())
                .collect()
        })
        .collect();
    rows.sort();
    rows
}

#[test]
fn tables_hold_the_same_rows_in_every_format() {
    let dir = tempfile::tempdir().unwrap();
    let sqlite = dir.path().join("bag.sqlite");
    convert(&sqlite, Format::Sqlite, Shape::Tables, Selection::all());
    let csv = dir.path().join("csv");
    convert(&csv, Format::Csv, Shape::Tables, Selection::all());
    let parquet = dir.path().join("parquet");
    convert(&parquet, Format::Parquet, Shape::Tables, Selection::all());
    for table in tables::tables(Selection::all()) {
        let mut expected = sqlite_rows(&sqlite, table);
        expected.sort();
        assert!(!expected.is_empty(), "{}", table.name);
        assert_eq!(
            parquet_rows(&parquet.join(format!("{}.parquet", table.name)), table),
            expected,
            "{}",
            table.name
        );
        let from_csv = csv_rows(&csv.join(format!("{}.csv", table.name)));
        assert_eq!(
            without_geometry(table, &from_csv),
            without_geometry(table, &expected),
            "{}",
            table.name
        );
    }
    let metadata: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(csv.join("metadata.json")).unwrap()).unwrap();
    assert_eq!(metadata["layout"], "tables");
    assert_eq!(metadata["reference_date"], "2026-10-07");
    assert!(
        fs::read_to_string(csv.join("panden.csv"))
            .unwrap()
            .contains("\"POLYGON ((155000 463000, 155100 463000")
    );
}

#[test]
fn excluded_types_leave_out_their_tables_and_unverifiable_links() {
    let dir = tempfile::tempdir().unwrap();
    let mut selection = Selection::all();
    selection.remove(ObjectType::Building);
    selection.remove(ObjectType::AddressableObject);
    let csv = dir.path().join("csv");
    convert(&csv, Format::Csv, Shape::Tables, selection);
    let mut files: Vec<_> = fs::read_dir(&csv)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    assert_eq!(
        files,
        [
            "gemeenten.csv",
            "metadata.json",
            "nummeraanduidingen.csv",
            "openbare_ruimten.csv",
            "woonplaatsen.csv"
        ]
    );
    // Without adresseerbare objecten, nummeraanduidingen keep no object link.
    let addresses = csv_rows(&csv.join("nummeraanduidingen.csv"));
    assert!(
        addresses
            .iter()
            .all(|row| row[8].is_empty() && row[9].is_empty())
    );
    let metadata = fs::read_to_string(csv.join("metadata.json")).unwrap();
    assert!(metadata.contains("\"woonplaatsen,gemeenten,openbare-ruimten,nummeraanduidingen\""));
}

#[test]
fn address_lists_match_across_formats() {
    let dir = tempfile::tempdir().unwrap();
    let paths: Vec<_> = [Format::Sqlite, Format::Csv, Format::Parquet]
        .into_iter()
        .map(|format| {
            let path = dir
                .path()
                .join(format!("adreslijst.{}", format.extension()));
            convert(&path, format, Shape::Addresses, Selection::all());
            path
        })
        .collect();
    let table = &bagkit::dataset::ADDRESS_LIST;
    let mut expected = sqlite_rows(&paths[0], table);
    expected.sort();
    assert_eq!(expected.len(), 4);
    assert_eq!(csv_rows(&paths[1]), expected);
    assert_eq!(parquet_rows(&paths[2], table), expected);
}

#[cfg(unix)]
#[test]
fn outputs_get_the_permissions_of_new_files() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("adreslijst.csv");
    convert(&file, Format::Csv, Shape::Addresses, Selection::all());
    let tables = dir.path().join("tables");
    convert(&tables, Format::Csv, Shape::Tables, Selection::all());
    // A new file created with mode 0o666 and the current umask, for comparison.
    let reference = dir.path().join("reference");
    fs::File::create(&reference).unwrap();
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&file), mode(&reference));
    assert_eq!(
        mode(&tables.join("nummeraanduidingen.csv")),
        mode(&reference)
    );
}
