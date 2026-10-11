use crate::{
    api,
    convert::{self, Options, Shape},
    dataset::Format,
    extract::{Extract, ObjectType, Selection},
    query, server,
};
use anyhow::{Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    version,
    about = "Convert BAG extracts to SQLite, CSV or Parquet, and look addresses up"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Convert BAG XML, gzip, ZIPs (including nested national extracts) or directories.
    #[command(alias = "import")]
    Convert(ConvertArgs),
    /// Serve the HTTP API for address and nearest-address lookups from SQLite output.
    Serve(ServeArgs),
    /// Answer an HTTP API request without running a server.
    Query {
        /// A SQLite output.
        #[arg(long, default_value = "bag.sqlite", value_name = "PATH")]
        database: PathBuf,
        /// A request path and query, for example
        /// /v1/adressen?postcode=9902HA&huisnummer=1.
        target: String,
    },
}

#[derive(Parser)]
pub struct ConvertArgs {
    /// XML, .gz or .zip files, or directories searched recursively for them.
    #[arg(required = true)]
    pub inputs: Vec<PathBuf>,
    /// The output file, or directory for tables as CSV or Parquet. Defaults to bag.sqlite,
    /// bag (a directory) or adreslijst.csv/.parquet.
    #[arg(short, long, value_name = "PATH")]
    pub output: Option<PathBuf>,
    /// The output format.
    #[arg(long, value_enum, default_value_t = FormatArg::Sqlite)]
    pub format: FormatArg,
    /// tables: one dataset per object type; addresses: the adreslijst, one row per current
    /// adres that is not ingetrokken, with its openbare ruimte, woonplaats, gemeente,
    /// provincie, type of adresseerbaar object and WGS84 punt.
    #[arg(long, value_enum, default_value_t = ShapeArg::Tables)]
    pub shape: ShapeArg,
    /// Object types to convert, comma-separated: woonplaatsen, gemeenten,
    /// openbare-ruimten, nummeraanduidingen, adresseerbare-objecten, panden, and geometrie
    /// (the polygons of panden and adresseerbare objecten). Defaults to all of them. Tables
    /// shape only.
    #[arg(long, value_delimiter = ',', value_name = "TYPES")]
    pub include: Vec<String>,
    /// Object types to leave out, as for --include.
    #[arg(long, value_delimiter = ',', value_name = "TYPES")]
    pub exclude: Vec<String>,
    /// The peildatum: the date voorkomens must be valid on (defaults to today's local date).
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub date: Option<String>,
    /// Atomically replace an existing output after a successful conversion.
    #[arg(long)]
    pub force: bool,
    /// SQLite page cache in MiB; sorting uses temporary files.
    #[arg(long, default_value_t = 64, value_name = "MIB")]
    pub cache_mib: u32,
    /// Threads that inflate and parse XML; 0 uses every available core.
    #[arg(long, default_value_t = 0, value_name = "N")]
    pub threads: usize,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum FormatArg {
    Sqlite,
    Csv,
    Parquet,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ShapeArg {
    Tables,
    Addresses,
}

/// The object types and whether geometrie is included, from `--include` and
/// `--exclude`.
fn selection(include: &[String], exclude: &[String]) -> Result<(Selection, bool)> {
    let mut selection = Selection::default();
    let mut footprints = include.is_empty();
    if include.is_empty() {
        selection = Selection::all();
    }
    for (names, wanted) in [(include, true), (exclude, false)] {
        for name in names {
            if name == "geometrie" {
                footprints = wanted;
                continue;
            }
            let Some(object_type) = ObjectType::from_name(name) else {
                let known: Vec<_> = ObjectType::ALL.iter().map(|t| t.name()).collect();
                bail!(
                    "unknown object type {name:?}; use {} or geometrie",
                    known.join(", ")
                );
            };
            if wanted {
                selection.insert(object_type);
            } else {
                selection.remove(object_type);
            }
        }
    }
    Ok((selection, footprints))
}

#[derive(Parser)]
#[command(
    version,
    about = "Serve the HTTP API for address and nearest-address lookups"
)]
pub struct ServeArgs {
    /// The port to listen on.
    #[arg(default_value_t = 8080)]
    pub port: u16,
    /// A SQLite output.
    #[arg(long, default_value = "bag.sqlite", value_name = "PATH")]
    pub database: PathBuf,
    /// The address to bind.
    #[arg(long, default_value = "0.0.0.0")]
    pub host: String,
    /// Database workers answering queries.
    #[arg(long, default_value_t = 4, value_name = "N")]
    pub workers: usize,
    /// Maximum accepted connections; excess sockets are closed immediately.
    #[arg(long, default_value_t = 256, value_name = "N")]
    pub max_connections: usize,
    /// Total connection lifetime, including headers, queries and response writes.
    #[arg(long, default_value_t = 30, value_name = "SECONDS")]
    pub connection_timeout_secs: u64,
}

pub fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Convert(args) => run_convert(args),
        Command::Serve(args) => run_serve(args),
        Command::Query { database, target } => {
            let response = api::get(&query::open(&database)?, &target)?;
            let body: serde_json::Value = serde_json::from_str(&response.body)?;
            let body = serde_json::to_string_pretty(&body)?;
            if response.status >= 400 {
                bail!(
                    "the request failed with status {}:\n{body}",
                    response.status
                );
            }
            println!("{body}");
            Ok(())
        }
    }
}

fn run_convert(args: ConvertArgs) -> Result<()> {
    let format = match args.format {
        FormatArg::Sqlite => Format::Sqlite,
        FormatArg::Csv => Format::Csv,
        FormatArg::Parquet => Format::Parquet,
    };
    let shape = match args.shape {
        ShapeArg::Tables => Shape::Tables,
        ShapeArg::Addresses => Shape::Addresses,
    };
    if shape == Shape::Addresses && !(args.include.is_empty() && args.exclude.is_empty()) {
        bail!("--include and --exclude apply to the tables shape");
    }
    let (selection, footprints) = selection(&args.include, &args.exclude)?;
    let output = args.output.unwrap_or_else(|| {
        PathBuf::from(match (shape, format) {
            (_, Format::Sqlite) => "bag.sqlite".into(),
            (Shape::Tables, _) => "bag".into(),
            (Shape::Addresses, format) => format!("adreslijst.{}", format.extension()),
        })
    });
    let date = args
        .date
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());
    let mut extract = Extract::new(args.inputs, &date)?;
    extract.threads = args.threads;
    extract.footprints = footprints;
    let options = Options {
        output,
        force: args.force,
        format,
        shape,
        selection,
        cache_mib: args.cache_mib,
    };
    let stats = convert::convert(&extract, &options)?;
    let phases: Vec<_> = stats
        .phases
        .iter()
        .map(|(name, duration)| format!("{name} {:.1}s", duration.as_secs_f64()))
        .collect();
    let rows = stats
        .rows
        .map_or(String::new(), |rows| format!(" ({rows} addresses)"));
    eprintln!(
        "Converted {} of {} voorkomens in {:.3}s into {}{rows} ({})",
        stats.read.current,
        stats.read.records,
        stats.elapsed.as_secs_f64(),
        options.output.display(),
        phases.join(", ")
    );
    if stats.points_outside_rd_new > 0 {
        eprintln!(
            "{} adresseerbare objecten lie outside RD New and have no WGS84 coordinates",
            stats.points_outside_rd_new
        );
    }
    Ok(())
}

fn run_serve(args: ServeArgs) -> Result<()> {
    server::serve(
        &args.database,
        &format!("{}:{}", args.host, args.port),
        args.workers,
        args.max_connections,
        std::time::Duration::from_secs(args.connection_timeout_secs),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &str) -> Vec<String> {
        list.split(',')
            .filter(|n| !n.is_empty())
            .map(Into::into)
            .collect()
    }

    #[test]
    fn include_and_exclude_choose_object_types_and_footprints() {
        assert_eq!(selection(&[], &[]).unwrap(), (Selection::all(), true));
        let (chosen, footprints) = selection(&[], &names("panden,geometrie")).unwrap();
        assert!(!chosen.contains(ObjectType::Building) && chosen.contains(ObjectType::Address));
        assert!(!footprints);
        let (chosen, footprints) =
            selection(&names("nummeraanduidingen,adresseerbare-objecten"), &[]).unwrap();
        assert_eq!(
            chosen.types().collect::<Vec<_>>(),
            [ObjectType::Address, ObjectType::AddressableObject]
        );
        assert!(!footprints, "geometrie must be listed when including");
        assert!(
            selection(&names("adresseerbare-objecten,geometrie"), &[])
                .unwrap()
                .1
        );
        let error = selection(&names("woningen"), &[]).unwrap_err();
        assert!(error.to_string().contains("unknown object type"), "{error}");
    }
}
