//! Converting an extract: read the selected object types, shape them into rows, and write
//! them in a format to a destination that is replaced only once the output is complete.
//!
//! ```no_run
//! use bagkit::{
//!     convert::{self, Options, Shape},
//!     dataset::Format,
//!     extract::{Extract, Selection},
//! };
//!
//! let extract = Extract::new(vec!["lvbag-extract-nl.zip".into()], "2026-10-08")?;
//! let options = Options {
//!     output: "adreslijst.parquet".into(),
//!     force: true,
//!     format: Format::Parquet,
//!     shape: Shape::Addresses,
//!     selection: Selection::all(),
//!     cache_mib: 64,
//! };
//! convert::convert(&extract, &options)?;
//! # anyhow::Ok(())
//! ```
use crate::{
    address_list,
    dataset::{ADDRESS_LIST, Extents, Format, Table},
    extract::{Extract, ReadStats, Selection},
    municipalities,
    output::{CsvWriter, Destination, Target, Writer},
    tables,
};
use anyhow::{Result, bail, ensure};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

/// How voorkomens become rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// One dataset per object type, as the SQLite core schema has them. CSV and Parquet
    /// write a directory with one file per table.
    Tables,
    /// The adreslijst: one row per current adres that is not ingetrokken, with its names
    /// and punt; see [`crate::address_list`]. Always one file.
    Addresses,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub output: PathBuf,
    /// Replace an existing output once the new one is complete.
    pub force: bool,
    pub format: Format,
    pub shape: Shape,
    /// Object types to read and write, for the tables shape. The adreslijst always
    /// reads what it needs.
    pub selection: Selection,
    /// SQLite's page cache while building, in MiB.
    pub cache_mib: u32,
}

#[derive(Debug, Default)]
pub struct Stats {
    pub read: ReadStats,
    /// Rows of an adreslijst.
    pub rows: Option<u64>,
    /// Adresseerbare objecten whose punt lies outside RD New's area, and so have no WGS84
    /// coordinates.
    pub points_outside_rd_new: u64,
    pub phases: Vec<(&'static str, Duration)>,
    pub elapsed: Duration,
}

/// What a shape wrote, beyond its rows.
struct Shaped {
    read: ReadStats,
    rows: Option<u64>,
    points_outside_rd_new: u64,
    /// Used by SQLite's GeoPackage registration.
    #[cfg_attr(not(feature = "sqlite"), allow(dead_code))]
    extents: Extents,
}

/// Converts `extract` as `options` say.
pub fn convert(extract: &Extract, options: &Options) -> Result<Stats> {
    let started = Instant::now();
    let destination = Destination {
        path: options.output.clone(),
        force: options.force,
    };
    destination.check()?;
    ensure!(options.cache_mib > 0, "cache size must be positive");
    let selection = match options.shape {
        Shape::Tables => options.selection,
        Shape::Addresses => address_list::selection(),
    };
    ensure!(!selection.is_empty(), "select at least one object type");
    let run_shape = |writer: &mut Writer| -> Result<Shaped> {
        Ok(match options.shape {
            Shape::Tables => {
                let written = tables::write(extract, selection, writer)?;
                Shaped {
                    read: written.read,
                    rows: None,
                    points_outside_rd_new: written.points_outside_rd_new,
                    extents: written.extents,
                }
            }
            Shape::Addresses => {
                let written = address_list::write(extract, writer)?;
                Shaped {
                    read: written.read,
                    rows: Some(written.rows),
                    points_outside_rd_new: written.points_outside_rd_new,
                    extents: Vec::new(),
                }
            }
        })
    };
    let tables: Vec<&'static Table> = match options.shape {
        Shape::Tables => tables::tables(selection),
        Shape::Addresses => vec![&ADDRESS_LIST],
    };
    let mut phases = Vec::new();
    let mut phase = Instant::now();
    let shaped = match (options.format, options.shape) {
        #[cfg(feature = "sqlite")]
        (Format::Sqlite, shape) => {
            let db = crate::sqlite::Database::create(&destination, options.cache_mib)?;
            // Every table exists in a database, so its views and lookups work; tables of
            // unselected types stay empty.
            let created = match shape {
                Shape::Tables => tables::tables(Selection::all()),
                Shape::Addresses => tables.clone(),
            };
            let mut writer = Writer::Sqlite(crate::sqlite::Writer::new(&db.connection, &created)?);
            let shaped = run_shape(&mut writer)?;
            drop(writer);
            phases.push(("load", phase.elapsed()));
            phase = Instant::now();
            match shape {
                Shape::Tables => crate::sqlite::finish_tables(
                    &db.connection,
                    extract.footprints.then_some(&shaped.extents),
                )?,
                Shape::Addresses => db.connection.execute_batch(
                    "CREATE INDEX adreslijst_by_postcode
                         ON adreslijst (postcode, huisnummer);",
                )?,
            }
            db.metadata(&metadata(extract, options, selection, &shaped.read))?;
            phases.push(("index", phase.elapsed()));
            phase = Instant::now();
            db.publish(&destination)?;
            shaped
        }
        (Format::Csv | Format::Parquet, Shape::Tables) => {
            let directory = destination.temporary_directory()?;
            let mut writer =
                file_writer(options.format, &tables, Target::Directory(directory.path()))?;
            let shaped = run_shape(&mut writer)?;
            drop(writer);
            let entries = metadata(extract, options, selection, &shaped.read);
            let json: serde_json::Map<String, serde_json::Value> = entries
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value.into()))
                .collect();
            std::fs::write(
                directory.path().join("metadata.json"),
                serde_json::to_string_pretty(&json)? + "\n",
            )?;
            phases.push(("load", phase.elapsed()));
            phase = Instant::now();
            destination.publish_directory(directory)?;
            shaped
        }
        (Format::Csv | Format::Parquet, Shape::Addresses) => {
            let file = destination.temporary_file(&format!(".{}", options.format.extension()))?;
            let mut writer = file_writer(options.format, &tables, Target::File(file.as_file()))?;
            let shaped = run_shape(&mut writer)?;
            drop(writer);
            phases.push(("load", phase.elapsed()));
            phase = Instant::now();
            destination.publish_file(file)?;
            shaped
        }
        #[allow(unreachable_patterns)]
        (format, _) => bail!("bagkit was built without {format:?} support"),
    };
    phases.push(("publish", phase.elapsed()));
    Ok(Stats {
        read: shaped.read,
        rows: shaped.rows,
        points_outside_rd_new: shaped.points_outside_rd_new,
        phases,
        elapsed: started.elapsed(),
    })
}

/// A CSV or Parquet writer for `tables`.
fn file_writer(
    format: Format,
    tables: &[&'static Table],
    target: Target,
) -> Result<Writer<'static>> {
    Ok(match format {
        Format::Csv => Writer::Csv(CsvWriter::new(tables, target)?),
        #[cfg(feature = "parquet")]
        Format::Parquet => Writer::Parquet(crate::parquet::Writer::new(tables, target)?),
        format => bail!("bagkit was built without {format:?} support"),
    })
}

/// Every metadata key an output may record, with what it means. The documentation's
/// metadata reference is generated from this list.
pub const METADATA: &[(&str, &str)] = &[
    (
        "schema_version",
        "The layout version. `query` and `serve` refuse databases of another version",
    ),
    ("layout", "`tables` or `address_list` (an adreslijst)"),
    (
        "reference_date",
        "The peildatum voorkomens were selected for",
    ),
    ("version", "The bagkit version that wrote the output"),
    (
        "municipality_names",
        "Where bagkit's municipality table came from, and when",
    ),
    ("object_types", "The object types that were read"),
    (
        "geometry",
        "`included` or `omitted`: whether geometrie was read",
    ),
    (
        "extract_date",
        "The extract's `StandTechnischeDatum`, when the inputs declare one date",
    ),
];

/// What an output records about itself; every key is in [`METADATA`].
fn metadata(
    extract: &Extract,
    options: &Options,
    selection: Selection,
    read: &ReadStats,
) -> Vec<(&'static str, String)> {
    let mut entries = vec![
        ("schema_version", crate::SCHEMA_VERSION.to_owned()),
        (
            "layout",
            match options.shape {
                Shape::Tables => "tables",
                Shape::Addresses => "address_list",
            }
            .to_owned(),
        ),
        ("reference_date", extract.date.clone()),
        ("version", env!("CARGO_PKG_VERSION").to_owned()),
        ("municipality_names", municipalities::SOURCE.to_owned()),
        (
            "object_types",
            selection
                .types()
                .map(|t| t.name())
                .collect::<Vec<_>>()
                .join(","),
        ),
        (
            "geometry",
            if extract.footprints && options.shape == Shape::Tables {
                "included"
            } else {
                "omitted"
            }
            .to_owned(),
        ),
    ];
    match read.extract_dates.len() {
        0 => (),
        1 => entries.push(("extract_date", read.extract_dates.first().unwrap().clone())),
        _ => eprintln!("Inputs declare different extract dates; not recording one"),
    }
    entries
}
