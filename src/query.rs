use crate::{SCHEMA_VERSION, geo::wgs84_to_rd};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Row, params_from_iter, types::Value};
use std::{
    collections::{HashMap, hash_map::Entry},
    path::Path,
};

pub struct Database {
    connection: Connection,
    /// An adreslijst (`convert --shape addresses`) rather than a database with tables.
    address_list: bool,
}

impl std::ops::Deref for Database {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        &self.connection
    }
}

impl Database {
    /// Whether this is an adreslijst, which answers address lookups only.
    pub fn is_address_list(&self) -> bool {
        self.address_list
    }

    /// A value from `bagkit_metadata`, if the output recorded it.
    pub fn metadata(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .query_row(
                "SELECT value FROM bagkit_metadata WHERE key = ?",
                [key],
                |r| r.get(0),
            )
            .optional()?)
    }
}

pub fn open(path: &Path) -> Result<Database> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.execute_batch(
        "PRAGMA query_only = ON;
         PRAGMA cache_size = -8192;
         PRAGMA mmap_size = 268435456;",
    )?;
    db.set_prepared_statement_cache_capacity(8);
    let version: String = db
        .query_row(
            "SELECT value FROM bagkit_metadata WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .ok()
        .with_context(|| {
            format!(
                "{} is not a bagkit database; import the BAG extract again",
                path.display()
            )
        })?;
    ensure!(
        version == SCHEMA_VERSION,
        "{} uses schema version {version}, but this bagkit reads version {SCHEMA_VERSION}; import the BAG extract again",
        path.display()
    );
    let layout: Option<String> = db
        .query_row(
            "SELECT value FROM bagkit_metadata WHERE key = 'layout'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(Database {
        connection: db,
        address_list: layout.as_deref() == Some("address_list"),
    })
}

/// How a house letter or addition must match.
#[derive(Debug, Clone, PartialEq)]
pub enum Match {
    /// Any value, or none.
    Any,
    /// No value.
    Absent,
    /// This value, compared case-insensitively.
    Equals(String),
}

/// Adressen with a postcode, optionally narrowed to a huisnummer, huisletter and
/// huisnummertoevoeging.
#[derive(Debug, Clone)]
pub struct AddressQuery {
    pub postcode: String,
    /// The huisnummer.
    pub house_number: Option<i64>,
    /// The huisletter.
    pub house_letter: Match,
    /// The huisnummertoevoeging.
    pub house_number_addition: Match,
}

/// An adres: a nummeraanduiding with its openbare ruimte, woonplaats and adresseerbaar
/// object. Fields an adreslijst does not store are `None`.
#[derive(Debug, Clone)]
pub struct Address {
    /// The nummeraanduiding's identificatie.
    pub id: i64,
    pub postcode: Option<String>,
    /// The huisnummer.
    pub house_number: i64,
    /// The huisletter.
    pub house_letter: Option<String>,
    /// The huisnummertoevoeging.
    pub house_number_addition: Option<String>,
    /// The openbare ruimte's naam.
    pub street: String,
    /// The openbare ruimte's identificatie.
    pub public_space_id: i64,
    /// The woonplaats's naam: the nummeraanduiding's own, or else its openbare ruimte's.
    pub locality: String,
    /// The woonplaats's identificatie.
    pub locality_id: Option<i64>,
    /// The gemeente's naam.
    pub municipality: Option<String>,
    /// The provincie's naam.
    pub province: Option<String>,
    /// The nummeraanduiding's status; `None` from an adreslijst, which holds no ingetrokken
    /// nummeraanduidingen.
    pub status: Option<String>,
    /// The adresseerbaar object the nummeraanduiding belongs to.
    pub object: Option<AddressableObject>,
}

/// An adresseerbaar object, as an adres refers to it.
#[derive(Debug, Clone)]
pub struct AddressableObject {
    /// The identificatie.
    pub id: Option<i64>,
    /// The type: `Verblijfsobject`, `Ligplaats` or `Standplaats`.
    pub kind: String,
    pub status: Option<String>,
    /// Indicatie nevenadres: whether the adres is a nevenadres of the object rather than its
    /// hoofdadres.
    pub is_secondary_address: Option<bool>,
    /// The gebruiksdoelen.
    pub usage_purposes: Option<Vec<String>>,
    /// The oppervlakte in square metres.
    pub usable_area_m2: Option<i64>,
    /// The punt in RD New, when the database stores it.
    pub rd: Option<(f64, f64)>,
    /// The punt's WGS84 breedtegraad and lengtegraad; `None` outside RD New's area of use.
    pub lat_lon: Option<(f64, f64)>,
    /// The panden a verblijfsobject maakt deel uit van.
    pub buildings: Option<Vec<Building>>,
}

impl AddressableObject {
    /// The punt in RD New, from the stored punt or else converted from WGS84.
    pub fn rd_point(&self) -> Option<(f64, f64)> {
        self.rd
            .or_else(|| self.lat_lon.and_then(|(lat, lon)| wgs84_to_rd(lat, lon)))
    }
}

/// A pand.
#[derive(Debug, Clone)]
pub struct Building {
    /// The identificatie.
    pub id: i64,
    /// The oorspronkelijk bouwjaar.
    pub construction_year: i64,
    pub status: String,
}

const DETAIL_FIELDS: &str = "
    nummeraanduiding_identificatie,
    postcode,
    huisnummer,
    huisletter,
    huisnummertoevoeging,
    openbare_ruimte_naam,
    openbare_ruimte_identificatie,
    woonplaats_naam,
    woonplaats_identificatie,
    gemeente_naam,
    provincie_naam,
    nummeraanduiding_status,
    adresseerbaar_object_identificatie,
    type_adresseerbaar_object,
    adresseerbaar_object_status,
    indicatie_nevenadres,
    gebruiksdoelen,
    oppervlakte,
    rd_x,
    rd_y,
    breedtegraad,
    lengtegraad";

const LIST_FIELDS: &str = "
    nummeraanduiding_identificatie,
    postcode,
    huisnummer,
    huisletter,
    huisnummertoevoeging,
    openbare_ruimte_naam,
    openbare_ruimte_identificatie,
    woonplaats_naam,
    gemeente_naam,
    provincie_naam,
    type_adresseerbaar_object,
    breedtegraad,
    lengtegraad";

const NOT_WITHDRAWN: &str = "nummeraanduiding_status != 'Naamgeving ingetrokken'";

/// Searches start at 100 m and widen until the nearest hit lies within the search radius.
const FIRST_RADIUS_MM: i64 = 100_000;
const MAX_RADIUS_MM: i64 = 25_600_000;

/// Why an adreslijst cannot answer a nearest lookup.
pub const NEAREST_NEEDS_TABLES: &str =
    "nearest lookup needs tables; this database is an address list";

/// A lookup this database cannot answer, as opposed to one that failed.
#[derive(Debug)]
pub struct Unsupported(pub &'static str);

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for Unsupported {}

/// Current adressen whose nummeraanduiding is not ingetrokken, ordered by huisnummer,
/// huisletter, huisnummertoevoeging and identificatie.
pub fn find_addresses(db: &Database, query: &AddressQuery) -> Result<Vec<Address>> {
    let mut sql = if db.address_list {
        format!("SELECT{LIST_FIELDS} FROM adreslijst WHERE postcode = ?")
    } else {
        format!("SELECT{DETAIL_FIELDS} FROM adressen WHERE postcode = ? AND {NOT_WITHDRAWN}")
    };
    let mut parameters = vec![Value::Text(query.postcode.clone())];
    if let Some(number) = query.house_number {
        sql.push_str(" AND huisnummer = ?");
        parameters.push(Value::Integer(number));
    }
    for (column, value) in [
        ("huisletter", &query.house_letter),
        ("huisnummertoevoeging", &query.house_number_addition),
    ] {
        match value {
            Match::Any => (),
            Match::Absent => sql.push_str(&format!(" AND {column} IS NULL")),
            Match::Equals(value) => {
                sql.push_str(&format!(" AND {column} = ? COLLATE NOCASE"));
                parameters.push(Value::Text(value.clone()));
            }
        }
    }
    sql.push_str(
        " ORDER BY huisnummer, huisletter, huisnummertoevoeging, nummeraanduiding_identificatie",
    );
    addresses(db, &sql, parameters)
}

/// One adres by its nummeraanduiding's identificatie, including an ingetrokken one.
pub fn address(db: &Database, id: i64) -> Result<Option<Address>> {
    let sql = if db.address_list {
        format!("SELECT{LIST_FIELDS} FROM adreslijst WHERE nummeraanduiding_identificatie = ?")
    } else {
        format!("SELECT{DETAIL_FIELDS} FROM adressen WHERE nummeraanduiding_identificatie = ?")
    };
    Ok(addresses(db, &sql, vec![Value::Integer(id)])?.pop())
}

/// The current adressen of the adresseerbaar object nearest to an RD New point, hoofdadres
/// first. Ties go to the object with the smallest identificatie; nothing within
/// 25.6 km finds nothing.
pub fn nearest_addresses(db: &Database, x: f64, y: f64) -> Result<Vec<Address>> {
    ensure!(x.is_finite() && y.is_finite(), "coordinates must be finite");
    if db.address_list {
        return Err(Unsupported(NEAREST_NEEDS_TABLES).into());
    }
    let Some(object) = nearest(db, x, y)? else {
        return Ok(Vec::new());
    };
    let sql = format!(
        "SELECT{DETAIL_FIELDS}
         FROM adressen
         WHERE adresseerbaar_object_identificatie = ? AND {NOT_WITHDRAWN}
         ORDER BY indicatie_nevenadres, nummeraanduiding_identificatie"
    );
    addresses(db, &sql, vec![Value::Integer(object)])
}

/// The adresseerbaar object with a current nummeraanduiding nearest to an RD New point.
/// Ties go to the smallest identificatie.
fn nearest(db: &Database, x: f64, y: f64) -> Result<Option<i64>> {
    let (x, y) = ((x * 1000.0).round() as i64, (y * 1000.0).round() as i64);
    let mut statement = db.prepare_cached(
        "SELECT id, min_x, min_y
         FROM adresseerbare_objecten_rtree
         WHERE min_x >= ?1 AND max_x <= ?2 AND min_y >= ?3 AND max_y <= ?4",
    )?;
    let mut radius = FIRST_RADIUS_MM;
    loop {
        let mut best: Option<(i64, i64)> = None;
        let mut rows = statement.query([x - radius, x + radius, y - radius, y + radius])?;
        while let Some(row) = rows.next()? {
            let (id, px, py): (i64, i64, i64) = (row.get(0)?, row.get(1)?, row.get(2)?);
            let distance = (px - x).pow(2) + (py - y).pow(2);
            if best.is_none_or(|b| (distance, id) < (b.0, b.1)) {
                best = Some((distance, id));
            }
        }
        // Anything outside the box is farther than the radius, so a hit within it is final.
        if let Some((distance, id)) = best
            && distance <= radius * radius
        {
            return Ok(Some(id));
        }
        if radius >= MAX_RADIUS_MM {
            return Ok(None);
        }
        radius *= 2;
    }
}

fn addresses(db: &Database, sql: &str, parameters: Vec<Value>) -> Result<Vec<Address>> {
    let mut statement = db.prepare_cached(sql)?;
    let mut rows = statement.query(params_from_iter(parameters))?;
    let mut buildings: HashMap<i64, Vec<Building>> = HashMap::new();
    let mut results = Vec::new();
    while let Some(row) = rows.next()? {
        let mut address = if db.address_list {
            list_address(row)?
        } else {
            detail_address(row)?
        };
        if let Some(object) = &mut address.object
            && let Some(id) = object.id
        {
            let object_buildings = match buildings.entry(id) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => entry.insert(object_buildings(db, id)?),
            };
            object.buildings = Some(object_buildings.clone());
        }
        results.push(address);
    }
    Ok(results)
}

fn point(row: &Row, first: usize) -> Result<Option<(f64, f64)>> {
    Ok(row
        .get::<_, Option<f64>>(first)?
        .zip(row.get::<_, Option<f64>>(first + 1)?))
}

fn detail_address(row: &Row) -> Result<Address> {
    let object = match row.get::<_, Option<i64>>(12)? {
        Some(id) => {
            let usage_purposes: String = row.get(16)?;
            Some(AddressableObject {
                id: Some(id),
                kind: row.get(13)?,
                status: Some(row.get(14)?),
                is_secondary_address: row.get(15)?,
                usage_purposes: Some(serde_json::from_str(&usage_purposes)?),
                usable_area_m2: row.get(17)?,
                rd: point(row, 18)?,
                lat_lon: point(row, 20)?,
                buildings: None,
            })
        }
        None => None,
    };
    Ok(Address {
        id: row.get(0)?,
        postcode: row.get(1)?,
        house_number: row.get(2)?,
        house_letter: row.get(3)?,
        house_number_addition: row.get(4)?,
        street: row.get(5)?,
        public_space_id: row.get(6)?,
        locality: row.get(7)?,
        locality_id: row.get(8)?,
        municipality: row.get(9)?,
        province: row.get(10)?,
        status: Some(row.get(11)?),
        object,
    })
}

fn list_address(row: &Row) -> Result<Address> {
    let object = row
        .get::<_, Option<String>>(10)?
        .map(|kind| -> Result<_> {
            Ok(AddressableObject {
                id: None,
                kind,
                status: None,
                is_secondary_address: None,
                usage_purposes: None,
                usable_area_m2: None,
                rd: None,
                lat_lon: point(row, 11)?,
                buildings: None,
            })
        })
        .transpose()?;
    Ok(Address {
        id: row.get(0)?,
        postcode: row.get(1)?,
        house_number: row.get(2)?,
        house_letter: row.get(3)?,
        house_number_addition: row.get(4)?,
        street: row.get(5)?,
        public_space_id: row.get(6)?,
        locality: row.get(7)?,
        locality_id: None,
        municipality: row.get(8)?,
        province: row.get(9)?,
        status: None,
        object,
    })
}

fn object_buildings(db: &Database, object_id: i64) -> Result<Vec<Building>> {
    let mut statement = db.prepare_cached(
        "SELECT identificatie, oorspronkelijk_bouwjaar, status
         FROM verblijfsobject_panden
         JOIN panden ON identificatie = pand_identificatie
         WHERE verblijfsobject_identificatie = ?
         ORDER BY identificatie",
    )?;
    let buildings = statement
        .query_map([object_id], |r| {
            Ok(Building {
                id: r.get(0)?,
                construction_year: r.get(1)?,
                status: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(buildings)
}
