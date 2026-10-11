//! Datasets as shapes produce them: tables of typed columns, and rows of values.
//!
//! Every output format writes the same tables. Values are encoded for their format on
//! worker threads ([`Format::polygon`], [`Format::encode`]), so the writer only stores
//! them.
use crate::geometry::{RD_NEW, Ring, gpkg_polygon, wkb_polygon, wkt_polygon};
use std::io::Write;

/// Output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// A SQLite database; polygons as GeoPackage geometry.
    Sqlite,
    /// CSV with a header row; identificaties in official form, polygons as WKT.
    Csv,
    /// Parquet; polygons as GeoParquet WKB.
    Parquet,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Csv => "csv",
            Self::Parquet => "parquet",
        }
    }

    /// A polygon in RD New, encoded for this format.
    pub fn polygon(self, rings: &[Ring]) -> Value {
        match self {
            Self::Sqlite => Value::Blob(gpkg_polygon(rings, RD_NEW)),
            Self::Csv => Value::Text(wkt_polygon(rings)),
            Self::Parquet => Value::Blob(wkb_polygon(rings)),
        }
    }

    /// Rows of `table`, encoded for this format: CSV becomes text, others stay values.
    pub fn encode(self, table: &'static Table, rows: Vec<Row>) -> Encoded {
        match self {
            Self::Csv => {
                let mut text = Vec::with_capacity(rows.len() * 128);
                for row in &rows {
                    csv_row(&mut text, table, row);
                }
                Encoded {
                    table,
                    rows: rows.len(),
                    content: Content::Text(text),
                }
            }
            _ => Encoded {
                table,
                rows: rows.len(),
                content: Content::Rows(rows),
            },
        }
    }
}

/// How a column's values are typed and written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Integer,
    /// A BAG identificatie: an integer, written zero-padded to this many digits in CSV.
    Identifier(usize),
    /// A floating-point number, written with this many decimals in CSV.
    Real(usize),
    Text,
    /// A boolean, stored as 0 or 1 in SQLite.
    Boolean,
    /// A polygon in RD New, encoded per format.
    Geometry,
}

#[derive(Debug)]
pub struct Column {
    pub name: &'static str,
    pub kind: Kind,
    /// What the column holds, for the documentation.
    pub description: &'static str,
    /// Documented together with the column before it, which describes both.
    pub with_previous: bool,
    pub nullable: bool,
    /// The table an identificatie refers to, for SQLite foreign keys.
    pub references: Option<&'static str>,
    /// Compare case-insensitively in SQLite.
    pub nocase: bool,
}

impl Column {
    const fn new(name: &'static str, kind: Kind) -> Self {
        Self {
            name,
            kind,
            description: "",
            with_previous: false,
            nullable: false,
            references: None,
            nocase: false,
        }
    }

    const fn nullable(mut self) -> Self {
        self.nullable = true;
        self
    }

    const fn references(mut self, table: &'static str) -> Self {
        self.references = Some(table);
        self
    }

    const fn nocase(mut self) -> Self {
        self.nocase = true;
        self
    }

    const fn describe(mut self, description: &'static str) -> Self {
        self.description = description;
        self
    }

    const fn with_previous(mut self) -> Self {
        self.with_previous = true;
        self
    }
}

#[derive(Debug)]
pub struct Table {
    pub name: &'static str,
    /// What the table holds, for the documentation.
    pub description: &'static str,
    pub columns: &'static [Column],
    /// Leading columns that form the primary key.
    pub key: usize,
}

impl Table {
    pub fn column_names(&self) -> impl Iterator<Item = &'static str> {
        self.columns.iter().map(|c| c.name)
    }

    pub fn geometry_column(&self) -> Option<&'static str> {
        self.columns
            .iter()
            .find(|c| c.kind == Kind::Geometry)
            .map(|c| c.name)
    }
}

use Kind::{Boolean, Geometry, Identifier, Integer, Real, Text};

pub static LOCALITIES: Table = Table {
    name: "woonplaatsen",
    description: "Woonplaatsen.",
    key: 1,
    columns: &[
        Column::new("identificatie", Identifier(4)).describe("The woonplaatscode"),
        Column::new("naam", Text),
        Column::new("status", Text)
            .describe("`Woonplaats aangewezen` or `Woonplaats ingetrokken`"),
        Column::new("geconstateerd", Boolean),
        Column::new("gemeente_code", Identifier(4))
            .nullable()
            .references("gemeenten")
            .describe("From `GEM-WPL-RELATIE`, which Kadaster supplies alongside the BAG; `NULL` without it"),
    ],
};

pub static MUNICIPALITIES: Table = Table {
    name: "gemeenten",
    description: "Names and provinces from bagkit's built-in table, for the gemeenten in `GEM-WPL-RELATIE`.",
    key: 1,
    columns: &[
        Column::new("code", Identifier(4)).describe("The gemeentecode"),
        Column::new("naam", Text)
            .nullable()
            .describe("`NULL` for a code bagkit's table lacks"),
        Column::new("provincie_naam", Text).nullable(),
    ],
};

pub static PUBLIC_SPACES: Table = Table {
    name: "openbare_ruimten",
    description: "Openbare ruimten.",
    key: 1,
    columns: &[
        Column::new("identificatie", Identifier(16)),
        Column::new("naam", Text),
        Column::new("verkorte_naam", Text)
            .nullable()
            .describe("The NEN 5825 short name, for names longer than 24 characters"),
        Column::new("type", Text).describe("`Weg`, `Water`, and so on"),
        Column::new("status", Text).describe("`Naamgeving uitgegeven` or `Naamgeving ingetrokken`"),
        Column::new("woonplaats_identificatie", Identifier(4))
            .references("woonplaatsen")
            .describe("Ligt in"),
    ],
};

pub static ADDRESSES: Table = Table {
    name: "nummeraanduidingen",
    description: "Nummeraanduidingen, including withdrawn ones.",
    key: 1,
    columns: &[
        Column::new("identificatie", Identifier(16)),
        Column::new("postcode", Text).nullable(),
        Column::new("huisnummer", Integer),
        Column::new("huisletter", Text)
            .nullable()
            .nocase()
            .describe("Compared case-insensitively in SQLite"),
        Column::new("huisnummertoevoeging", Text)
            .nullable()
            .nocase()
            .describe("Compared case-insensitively in SQLite"),
        Column::new("status", Text)
            .describe("`Naamgeving uitgegeven` or `Naamgeving ingetrokken`"),
        Column::new("openbare_ruimte_identificatie", Identifier(16))
            .references("openbare_ruimten")
            .describe("Ligt aan"),
        Column::new("woonplaats_identificatie", Identifier(4))
            .nullable()
            .references("woonplaatsen")
            .describe("Ligt in; `NULL` means the openbare ruimte's woonplaats"),
        Column::new("adresseerbaar_object_identificatie", Identifier(16))
            .nullable()
            .references("adresseerbare_objecten")
            .describe("The object that has this nummeraanduiding as its hoofdadres or a nevenadres; `NULL` when none, or when objects were not read"),
        Column::new("indicatie_nevenadres", Boolean)
            .nullable()
            .describe("Whether it is a nevenadres of that object rather than its hoofdadres"),
    ],
};

pub static ADDRESSABLE_OBJECTS: Table = Table {
    name: "adresseerbare_objecten",
    description: "Verblijfsobjecten, ligplaatsen and standplaatsen.",
    key: 1,
    columns: &[
        Column::new("identificatie", Identifier(16)),
        Column::new("type", Text).describe("`Verblijfsobject`, `Ligplaats` or `Standplaats`"),
        Column::new("status", Text),
        Column::new("gebruiksdoelen", Text)
            .describe("A sorted JSON array, such as `[\"woonfunctie\"]`; empty for ligplaatsen and standplaatsen"),
        Column::new("oppervlakte", Integer)
            .nullable()
            .describe("Gebruiksoppervlakte in square metres, for verblijfsobjecten"),
        Column::new("vlak", Geometry)
            .nullable()
            .describe("The object's polygon, when it has one and geometry was read"),
        Column::new("rd_x", Real(3)).nullable().describe(
            "Derived: the object's punt, or its vlak's area-weighted centroid, in RD New",
        ),
        Column::new("rd_y", Real(3)).nullable().with_previous(),
        Column::new("breedtegraad", Real(7))
            .nullable()
            .describe("Derived: the point in WGS84; `NULL` outside RD New's area of use"),
        Column::new("lengtegraad", Real(7)).nullable().with_previous(),
    ],
};

pub static BUILDINGS: Table = Table {
    name: "panden",
    description: "Panden.",
    key: 1,
    columns: &[
        Column::new("identificatie", Identifier(16)),
        Column::new("oorspronkelijk_bouwjaar", Integer),
        Column::new("status", Text),
        Column::new("geometrie", Geometry)
            .nullable()
            .describe("`NULL` when geometry was not read"),
    ],
};

pub static ADDRESSABLE_OBJECT_BUILDINGS: Table = Table {
    name: "verblijfsobject_panden",
    description: "Which panden each verblijfsobject maakt deel uit van. Written only when both adresseerbare objecten and panden are read.",
    key: 2,
    columns: &[
        Column::new("verblijfsobject_identificatie", Identifier(16))
            .references("adresseerbare_objecten"),
        Column::new("pand_identificatie", Identifier(16)).references("panden"),
    ],
};

/// The adreslijst: one row per current adres that is not ingetrokken; see
/// [`crate::address_list`].
pub static ADDRESS_LIST: Table = Table {
    name: "adreslijst",
    description: "One row per adres that is current on the peildatum and not ingetrokken.",
    key: 1,
    columns: &[
        Column::new("nummeraanduiding_identificatie", Identifier(16)),
        Column::new("openbare_ruimte_naam", Text),
        Column::new("huisnummer", Integer),
        Column::new("huisletter", Text).nullable().nocase(),
        Column::new("huisnummertoevoeging", Text)
            .nullable()
            .nocase(),
        Column::new("postcode", Text).nullable(),
        Column::new("woonplaats_naam", Text).describe(
            "The effective woonplaats: the nummeraanduiding's own, or else its openbare ruimte's",
        ),
        Column::new("gemeente_naam", Text)
            .nullable()
            .describe("The woonplaats's gemeente"),
        Column::new("provincie_naam", Text)
            .nullable()
            .describe("The gemeente's provincie"),
        Column::new("type_adresseerbaar_object", Text)
            .nullable()
            .describe("`Verblijfsobject`, `Ligplaats` or `Standplaats`; empty without an object"),
        Column::new("breedtegraad", Real(7))
            .nullable()
            .describe("The object's point in WGS84; nevenadressen share it"),
        Column::new("lengtegraad", Real(7))
            .nullable()
            .with_previous(),
        Column::new("openbare_ruimte_identificatie", Identifier(16)),
    ],
};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Value {
    pub fn text(value: Option<String>) -> Self {
        value.map_or(Self::Null, Self::Text)
    }

    pub fn integer(value: Option<i64>) -> Self {
        value.map_or(Self::Null, Self::Integer)
    }

    pub fn real(value: Option<f64>) -> Self {
        value.map_or(Self::Null, Self::Real)
    }

    pub fn boolean(value: bool) -> Self {
        Self::Integer(value.into())
    }
}

pub type Row = Vec<Value>;

/// Geometrie extents per table, `[min_x, min_y, max_x, max_y]`, in RD New; `None` for a
/// table without geometrie.
pub type Extents = Vec<(&'static str, Option<[f64; 4]>)>;

/// Rows of one table, encoded by a worker for the writer.
pub struct Encoded {
    pub table: &'static Table,
    pub rows: usize,
    pub content: Content,
}

pub enum Content {
    /// Values, for writers that store them typed.
    Rows(Vec<Row>),
    /// Rows formatted as CSV lines.
    Text(Vec<u8>),
}

/// Appends one row as a CSV line. Fields are quoted only when they contain a comma, quote
/// or line break.
fn csv_row(text: &mut Vec<u8>, table: &Table, row: &Row) {
    for (index, (column, value)) in table.columns.iter().zip(row).enumerate() {
        if index > 0 {
            text.push(b',');
        }
        let _ = match (column.kind, value) {
            (_, Value::Null) => Ok(()),
            (Identifier(digits), Value::Integer(id)) => write!(text, "{id:0digits$}"),
            (Boolean, Value::Integer(b)) => {
                text.extend_from_slice(if *b != 0 { b"true" } else { b"false" });
                Ok(())
            }
            (Real(decimals), Value::Real(r)) => write!(text, "{r:.decimals$}"),
            (_, Value::Integer(i)) => write!(text, "{i}"),
            (_, Value::Real(r)) => write!(text, "{r}"),
            (_, Value::Text(t)) => {
                csv_field(text, t);
                Ok(())
            }
            (_, Value::Blob(_)) => Ok(()),
        };
    }
    text.push(b'\n');
}

fn csv_field(text: &mut Vec<u8>, value: &str) {
    if value.contains([',', '"', '\n', '\r']) {
        text.push(b'"');
        text.extend_from_slice(value.replace('"', "\"\"").as_bytes());
        text.push(b'"');
    } else {
        text.extend_from_slice(value.as_bytes());
    }
}
