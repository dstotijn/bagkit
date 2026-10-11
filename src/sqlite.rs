//! SQLite output: a database built in a disposable file beside its destination, then
//! indexed and published.
use crate::{
    dataset::{Content, Encoded, Extents, Kind, Table, Value},
    geometry::RD_NEW,
    municipalities, rtree,
};
use anyhow::{Result, bail};
use rusqlite::{
    Connection, Statement, ToSql, params,
    types::{ToSqlOutput, ValueRef},
};
use std::collections::HashMap;

impl ToSql for Value {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Borrowed(match self {
            Value::Null => ValueRef::Null,
            Value::Integer(i) => ValueRef::Integer(*i),
            Value::Real(r) => ValueRef::Real(*r),
            Value::Text(t) => ValueRef::Text(t.as_bytes()),
            Value::Blob(b) => ValueRef::Blob(b),
        }))
    }
}

/// A database being built: journaling, syncing and foreign keys are off until
/// [`Database::publish`].
pub struct Database {
    pub connection: Connection,
    temp: tempfile::NamedTempFile,
}

impl Database {
    pub fn create(destination: &crate::output::Destination, cache_mib: u32) -> Result<Self> {
        let temp = destination.temporary_file(".sqlite")?;
        let connection = Connection::open(temp.path())?;
        connection.execute_batch(&format!(
            // Large pages mean fewer writes and shallower B-trees; warm lookups are unaffected.
            "PRAGMA page_size = 65536;
             PRAGMA journal_mode = OFF;
             PRAGMA synchronous = OFF;
             PRAGMA foreign_keys = OFF;
             PRAGMA temp_store = FILE;
             PRAGMA cache_size = -{};
             BEGIN;",
            u64::from(cache_mib) * 1024
        ))?;
        Ok(Self { connection, temp })
    }

    /// Commits, analyzes, and moves the database into place.
    pub fn publish(self, destination: &crate::output::Destination) -> Result<()> {
        // Sampled statistics are enough for the planner and avoid full scans.
        self.connection.execute_batch(
            "COMMIT;
             PRAGMA analysis_limit = 1000;
             ANALYZE;
             PRAGMA journal_mode = DELETE;
             PRAGMA synchronous = FULL;",
        )?;
        self.connection.close().map_err(|(_, e)| e)?;
        destination.publish_file(self.temp)
    }

    /// Records key-value metadata.
    pub fn metadata(&self, entries: &[(&str, String)]) -> Result<()> {
        self.connection.execute_batch(
            "CREATE TABLE bagkit_metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        for (key, value) in entries {
            self.connection.execute(
                "INSERT INTO bagkit_metadata VALUES (?1, ?2)",
                params![key, value],
            )?;
        }
        Ok(())
    }
}

/// `CREATE TABLE` for a table definition.
pub fn create_table(table: &Table) -> String {
    let columns: Vec<String> = table
        .columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let kind = match column.kind {
                Kind::Integer | Kind::Identifier(_) | Kind::Boolean => "INTEGER",
                Kind::Real(_) => "REAL",
                Kind::Text => "TEXT",
                Kind::Geometry => "BLOB",
            };
            let mut definition = format!("{} {kind}", column.name);
            if table.key == 1 && index == 0 {
                definition.push_str(" PRIMARY KEY");
            } else if !column.nullable {
                definition.push_str(" NOT NULL");
            }
            if column.nocase {
                definition.push_str(" COLLATE NOCASE");
            }
            if let Some(target) = column.references {
                definition.push_str(&format!(" REFERENCES {target}"));
            }
            definition
        })
        .collect();
    let key = if table.key > 1 {
        let key: Vec<_> = table.column_names().take(table.key).collect();
        format!(",\n    PRIMARY KEY ({})\n) WITHOUT ROWID", key.join(", "))
    } else {
        "\n)".into()
    };
    format!(
        "CREATE TABLE {} (\n    {}{key};",
        table.name,
        columns.join(",\n    ")
    )
}

/// Rows per multi-row INSERT statement.
const ROWS_PER_INSERT: usize = 128;

/// Buffers rows for one table and inserts them sorted by key, many rows per statement.
/// Within one statement SQLite keeps its cursor, so consecutive keys skip the B-tree
/// search; one statement per row would repeat it for every row. Duplicate keys are
/// ignored here; callers check identificaties themselves.
struct Inserter<'db> {
    table: &'static Table,
    many: Statement<'db>,
    one: Statement<'db>,
    rows: Vec<Vec<Value>>,
}

impl<'db> Inserter<'db> {
    fn new(db: &'db Connection, table: &'static Table) -> Result<Self> {
        let row = format!("({})", vec!["?"; table.columns.len()].join(", "));
        let insert = |rows: usize| {
            format!(
                "INSERT OR IGNORE INTO {} VALUES {}",
                table.name,
                vec![row.as_str(); rows].join(", ")
            )
        };
        Ok(Self {
            table,
            many: db.prepare(&insert(ROWS_PER_INSERT))?,
            one: db.prepare(&insert(1))?,
            rows: Vec::with_capacity(ROWS_PER_INSERT),
        })
    }

    fn push(&mut self, row: Vec<Value>) -> Result<()> {
        self.rows.push(row);
        if self.rows.len() == ROWS_PER_INSERT {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        let key = |row: &Vec<Value>| {
            let integer = |value: &Value| match value {
                Value::Integer(i) => *i,
                _ => 0,
            };
            (
                integer(&row[0]),
                row.get(1).filter(|_| self.table.key > 1).map_or(0, integer),
            )
        };
        self.rows.sort_unstable_by_key(key);
        if self.rows.len() == ROWS_PER_INSERT {
            for (index, value) in self.rows.iter().flatten().enumerate() {
                self.many.raw_bind_parameter(index + 1, value)?;
            }
            self.many.raw_execute()?;
        } else {
            for row in &self.rows {
                for (index, value) in row.iter().enumerate() {
                    self.one.raw_bind_parameter(index + 1, value)?;
                }
                self.one.raw_execute()?;
            }
        }
        self.rows.clear();
        Ok(())
    }
}

/// Inserts encoded rows into their tables.
pub struct Writer<'db> {
    inserters: HashMap<&'static str, Inserter<'db>>,
}

impl<'db> Writer<'db> {
    /// Creates `tables` in `db` and prepares to insert into them.
    pub fn new(db: &'db Connection, tables: &[&'static Table]) -> Result<Self> {
        let mut inserters = HashMap::new();
        for &table in tables {
            db.execute_batch(&create_table(table))?;
            inserters.insert(table.name, Inserter::new(db, table)?);
        }
        Ok(Self { inserters })
    }

    pub fn write(&mut self, encoded: Encoded) -> Result<()> {
        let Some(inserter) = self.inserters.get_mut(encoded.table.name) else {
            bail!("no table {}", encoded.table.name);
        };
        let Content::Rows(rows) = encoded.content else {
            bail!("SQLite rows must stay values");
        };
        rows.into_iter().try_for_each(|row| inserter.push(row))
    }

    pub fn flush(&mut self) -> Result<()> {
        self.inserters.values_mut().try_for_each(Inserter::flush)
    }
}

/// Adds what lookups need to a database of tables: indexes, the `adressen` view, the
/// R*Tree over adresseerbare objecten, and GeoPackage registration of polygons with their
/// extents (`[min_x, min_y, max_x, max_y]` per table).
pub fn finish_tables(db: &Connection, footprints: Option<&Extents>) -> Result<()> {
    db.execute_batch(include_str!("indexes.sql"))?;
    db.execute_batch(include_str!("derived.sql"))?;
    rtree::bulk_load(db, "adresseerbare_objecten_rtree", index_entries(db)?)?;
    if let Some(extents) = footprints {
        db.execute_batch(include_str!("gpkg.sql"))?;
        for (table, bounds) in extents {
            let description = match *table {
                "panden" => "Panden with their geometrie",
                _ => {
                    "Verblijfsobjecten, ligplaatsen and standplaatsen, with their vlak when they have one"
                }
            };
            db.execute(
                "INSERT INTO gpkg_contents
                     (table_name, data_type, identifier, description, min_x, min_y, max_x, max_y, srs_id)
                 VALUES (?1, 'features', ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    table,
                    description,
                    bounds.map(|b| b[0]),
                    bounds.map(|b| b[1]),
                    bounds.map(|b| b[2]),
                    bounds.map(|b| b[3]),
                    RD_NEW
                ],
            )?;
        }
    }
    Ok(())
}

/// Inserts bagkit's gemeente names, and gemeentecodes it lacks without one.
pub fn insert_municipalities(db: &Connection, used: impl IntoIterator<Item = i64>) -> Result<()> {
    let mut insert = db.prepare("INSERT OR IGNORE INTO gemeenten VALUES (?1, ?2, ?3)")?;
    for m in municipalities::all() {
        insert.execute(params![m.id, m.name, m.province])?;
    }
    for id in used {
        insert.execute(params![id, None::<String>, None::<String>])?;
    }
    Ok(())
}

/// Punten of adresseerbare objecten with at least one current nummeraanduiding, in RD New
/// millimetres. Both tables are read sequentially: object identificaties from current
/// nummeraanduidingen are sorted, then merged with the punten in identificatie order.
fn index_entries(db: &Connection) -> Result<Vec<(i64, i32, i32)>> {
    let mut eligible = db
        .prepare(
            "SELECT adresseerbaar_object_identificatie
             FROM nummeraanduidingen
             WHERE adresseerbaar_object_identificatie IS NOT NULL
                 AND status != 'Naamgeving ingetrokken'",
        )?
        .query_map([], |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    eligible.sort_unstable();
    eligible.dedup();
    let mut entries = Vec::with_capacity(eligible.len());
    let mut next = eligible.iter().peekable();
    let mut statement = db.prepare(
        "SELECT identificatie, rd_x, rd_y
             FROM adresseerbare_objecten
             WHERE breedtegraad IS NOT NULL",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        while next.next_if(|&&e| e < id).is_some() {}
        if next.next_if_eq(&&id).is_some() {
            let (x, y): (f64, f64) = (row.get(1)?, row.get(2)?);
            entries.push((id, (x * 1000.0).round() as i32, (y * 1000.0).round() as i32));
        }
    }
    Ok(entries)
}
