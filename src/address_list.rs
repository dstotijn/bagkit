//! The `addresses` shape, the adreslijst: one row per current adres that is not
//! ingetrokken, with its openbare ruimte, woonplaats, gemeente, provincie, type of
//! adresseerbaar object and WGS84 punt.
//!
//! The first part keeps woonplaatsen, openbare ruimten and each nummeraanduiding's
//! adresseerbaar object in memory; panden are never read. In the second part, workers
//! resolve nummeraanduidingen against that state and encode the rows, so the writer only
//! stores them. The same import rules apply as for tables.
use crate::{
    dataset::{ADDRESS_LIST, Encoded, Format, Row, Value},
    derived::object_point,
    extract::{Converter, Extract, ObjectType, Part, ReadStats, Selection},
    ids::Ids,
    municipalities::{self, Municipality},
    output::Writer,
    xml::{self, Address, BagObject, ObjectKind, bag_id, check_main_address, object_type},
};
use anyhow::{Result, bail, ensure};
use std::{
    collections::{BTreeSet, HashMap, hash_map::Entry},
    sync::atomic::{AtomicBool, Ordering},
};

/// The object types an adreslijst reads.
pub fn selection() -> Selection {
    Selection::only(&[
        ObjectType::Locality,
        ObjectType::MunicipalityRelation,
        ObjectType::PublicSpace,
        ObjectType::Address,
        ObjectType::AddressableObject,
    ])
}

/// What an adreslijst conversion wrote.
#[derive(Debug, Default)]
pub struct Written {
    pub read: ReadStats,
    pub rows: u64,
    pub points_outside_rd_new: u64,
    pub municipalities: BTreeSet<i64>,
}

/// Reads the adreslijst's object types from `extract` and writes its rows to `writer`.
/// The extract's geometrie setting is ignored: punten come from vlakken regardless.
pub fn write(extract: &Extract, writer: &mut Writer) -> Result<Written> {
    let format = writer.format();
    let mut state = State::default();
    let mut read = extract.read(Part::First, selection(), &PlaceReader, &mut |place| {
        state.add(place)
    })?;
    let early = std::mem::take(&mut state.early);
    let points_outside_rd_new = state.points_outside_rd_new;
    let places = state.finish()?;

    let mut ids = Ids::default();
    let mut rows = 0;
    let mut store = |chunk: Chunk, writer: &mut Writer| {
        ids.extend(chunk.ids);
        rows += chunk.encoded.rows as u64;
        writer.write(chunk.encoded)
    };
    let mut early_rows = Vec::new();
    let mut early_ids = Vec::new();
    for address in early {
        let id = address.id;
        if let Some(row) = places.resolve(address)? {
            early_ids.push(id);
            early_rows.push(row);
        }
    }
    store(
        Chunk {
            ids: early_ids,
            encoded: format.encode(&ADDRESS_LIST, early_rows),
        },
        writer,
    )?;
    let converter = Addresses {
        format,
        places: &places,
    };
    read.add(
        extract.read(Part::Second, selection(), &converter, &mut |chunk| {
            store(chunk, writer)
        })?,
    );
    ensure!(
        read.documents > 0 && read.records > 0,
        "no BAG records found in the supplied inputs"
    );
    writer.finish()?;
    if let Err(id) = ids.sorted_unique() {
        bail!(
            "duplicate Nummeraanduiding identificatie {}",
            bag_id(id, 16)
        );
    }
    places.check_links()?;
    Ok(Written {
        read,
        rows,
        points_outside_rd_new,
        municipalities: places.municipalities,
    })
}

/// What the first part keeps of a current voorkomen.
enum Place {
    Locality(xml::Locality),
    Municipality(xml::LocalityMunicipality),
    PublicSpace(xml::PublicSpace),
    Object {
        id: i64,
        kind: ObjectKind,
        addresses: Vec<(i64, bool)>,
        has_point: bool,
        wgs84: Option<[f64; 2]>,
    },
    Address(Address),
}

struct PlaceReader;

impl Converter for PlaceReader {
    type State = ();
    type Item = Place;

    fn begin(&self) {}

    fn convert(
        &self,
        _: &mut (),
        object: BagObject,
        emit: &mut dyn FnMut(Place) -> Result<()>,
    ) -> Result<()> {
        emit(match object {
            BagObject::Locality(l) => Place::Locality(l),
            BagObject::LocalityMunicipality(m) => Place::Municipality(m),
            BagObject::PublicSpace(p) => Place::PublicSpace(p),
            BagObject::Address(a) => Place::Address(a),
            BagObject::AddressableObject(o) => {
                check_main_address(&o)?;
                let (point, wgs84) = object_point(o.geometry.as_ref());
                Place::Object {
                    id: o.id,
                    kind: o.kind,
                    addresses: o.addresses,
                    has_point: point.is_some(),
                    wgs84,
                }
            }
            BagObject::Building(_) => unreachable!("panden are not selected"),
        })
    }
}

/// A nummeraanduiding's adresseerbaar object: the nummeraanduiding, the object with its
/// type and [`MAIN_ADDRESS`] packed above its identificatie, and the object's WGS84 punt
/// (NaN when it has none).
#[derive(Clone, Copy)]
struct Link {
    address: i64,
    object: i64,
    lat: f64,
    lon: f64,
}

/// Marks a hoofdadres in [`Link::object`]; identificaties are below 2^51.
const MAIN_ADDRESS: i64 = 1 << 62;
/// Adresseerbaar object types packed into [`Link::object`].
const KIND_SHIFT: u32 = 60;
const KIND_MASK: i64 = 3 << KIND_SHIFT;

fn pack_kind(kind: ObjectKind) -> i64 {
    (match kind {
        ObjectKind::Unit => 1,
        ObjectKind::Mooring => 2,
        ObjectKind::Pitch => 3,
    }) << KIND_SHIFT
}

fn unpack_kind(object: i64) -> ObjectKind {
    match (object & KIND_MASK) >> KIND_SHIFT {
        1 => ObjectKind::Unit,
        2 => ObjectKind::Mooring,
        _ => ObjectKind::Pitch,
    }
}

/// First-part state, collected by the writer.
#[derive(Default)]
struct State {
    localities: HashMap<i64, String>,
    relations: Vec<xml::LocalityMunicipality>,
    public_spaces: HashMap<i64, (String, i64)>,
    links: Vec<Link>,
    /// Nummeraanduidingen found in first-part documents.
    early: Vec<Address>,
    points_outside_rd_new: u64,
}

impl State {
    fn add(&mut self, place: Place) -> Result<()> {
        match place {
            Place::Locality(l) => {
                ensure!(
                    self.localities.insert(l.id, l.name).is_none(),
                    "duplicate Woonplaats identificatie {}",
                    bag_id(l.id, 4)
                );
            }
            Place::Municipality(m) => self.relations.push(m),
            Place::PublicSpace(p) => match self.public_spaces.entry(p.id) {
                Entry::Occupied(_) => {
                    bail!(
                        "duplicate OpenbareRuimte identificatie {}",
                        bag_id(p.id, 16)
                    )
                }
                Entry::Vacant(entry) => {
                    entry.insert((p.name, p.locality_id));
                }
            },
            Place::Object {
                id,
                kind,
                addresses,
                has_point,
                wgs84,
            } => {
                self.points_outside_rd_new += u64::from(has_point && wgs84.is_none());
                let [lat, lon] = wgs84.unwrap_or([f64::NAN; 2]);
                for (address, main) in addresses {
                    let flag = if main { MAIN_ADDRESS } else { 0 };
                    self.links.push(Link {
                        address,
                        object: id | pack_kind(kind) | flag,
                        lat,
                        lon,
                    });
                }
            }
            Place::Address(a) => self.early.push(a),
        }
        Ok(())
    }

    /// Checks references and relations, and returns the state the second part reads.
    fn finish(mut self) -> Result<Resolver> {
        self.links.sort_unstable_by_key(|link| link.address);
        let object_id = |link: &Link| link.object & !(KIND_MASK | MAIN_ADDRESS);
        let (mut count, mut example) = (0u64, None);
        for pair in self.links.windows(2) {
            if pair[0].address == pair[1].address {
                // The same object listing a nummeraanduiding twice means the object itself
                // repeats.
                if object_id(&pair[0]) == object_id(&pair[1]) {
                    let id = object_id(&pair[0]);
                    bail!(
                        "duplicate {} identificatie {}",
                        object_type(id),
                        bag_id(id, 16)
                    );
                }
                count += 1;
                example.get_or_insert(pair[0].address);
            }
        }
        ensure!(
            count == 0,
            "{count} nummeraanduidingen belong to more than one adresseerbaar object, for example {}",
            example.map(|id| bag_id(id, 16)).unwrap_or_default()
        );
        let missing: BTreeSet<i64> = self
            .public_spaces
            .values()
            .map(|(_, locality)| *locality)
            .filter(|locality| !self.localities.contains_key(locality))
            .collect();
        if let Some(first) = missing.first() {
            bail!(
                "{} openbare ruimten refer to missing woonplaatsen, for example {}",
                self.public_spaces
                    .values()
                    .filter(|(_, l)| missing.contains(l))
                    .count(),
                bag_id(*first, 4)
            );
        }
        let assigned = municipalities::assign(&self.relations, self.localities.keys().copied())?;
        let names: HashMap<i64, Municipality> = municipalities::all().map(|m| (m.id, m)).collect();
        let localities = self
            .localities
            .into_iter()
            .map(|(id, name)| {
                let municipality = assigned.get(&id).and_then(|m| names.get(m)).copied();
                (id, LocalityNames { name, municipality })
            })
            .collect();
        Ok(Resolver {
            localities,
            public_spaces: self.public_spaces,
            matched: self.links.iter().map(|_| AtomicBool::new(false)).collect(),
            links: self.links,
            municipalities: assigned.into_values().collect(),
        })
    }
}

struct LocalityNames {
    name: String,
    municipality: Option<Municipality>,
}

/// What the second part resolves nummeraanduidingen against; read-only.
struct Resolver {
    localities: HashMap<i64, LocalityNames>,
    public_spaces: HashMap<i64, (String, i64)>,
    /// Sorted by nummeraanduiding.
    links: Vec<Link>,
    /// Which links a nummeraanduiding has claimed.
    matched: Vec<AtomicBool>,
    municipalities: BTreeSet<i64>,
}

impl Resolver {
    /// Resolves a current nummeraanduiding into its row, or `None` for an ingetrokken one,
    /// and marks the link to its adresseerbaar object as claimed.
    fn resolve(&self, address: Address) -> Result<Option<Row>> {
        let link = self
            .links
            .binary_search_by_key(&address.id, |link| link.address)
            .ok()
            .map(|index| {
                self.matched[index].store(true, Ordering::Relaxed);
                self.links[index]
            });
        let Some((street, public_space_locality)) =
            self.public_spaces.get(&address.public_space_id)
        else {
            bail!(
                "nummeraanduidingen refer to missing openbare ruimten, for example {}",
                bag_id(address.public_space_id, 16)
            );
        };
        let locality_id = address.locality_id.unwrap_or(*public_space_locality);
        let Some(locality) = self.localities.get(&locality_id) else {
            bail!(
                "nummeraanduidingen refer to missing woonplaatsen, for example {}",
                bag_id(locality_id, 4)
            );
        };
        if address.status == "Naamgeving ingetrokken" {
            return Ok(None);
        }
        let point = link.filter(|l| !l.lat.is_nan()).map(|l| [l.lat, l.lon]);
        let text = |value: Option<&str>| value.map_or(Value::Null, |v| Value::Text(v.into()));
        Ok(Some(vec![
            Value::Integer(address.id),
            Value::Text(street.clone()),
            Value::Integer(address.house_number),
            Value::text(address.house_letter),
            Value::text(address.house_number_addition),
            Value::text(address.postcode),
            Value::Text(locality.name.clone()),
            text(locality.municipality.map(|m| m.name)),
            text(locality.municipality.map(|m| m.province)),
            text(link.map(|l| unpack_kind(l.object).bag_name())),
            Value::real(point.map(|p| p[0])),
            Value::real(point.map(|p| p[1])),
            Value::Integer(address.public_space_id),
        ]))
    }

    /// Fails on object links to nummeraanduidingen that never came.
    fn check_links(&self) -> Result<()> {
        let count = self
            .matched
            .iter()
            .filter(|m| !m.load(Ordering::Relaxed))
            .count();
        if let Some(index) = self.matched.iter().position(|m| !m.load(Ordering::Relaxed)) {
            bail!(
                "{count} links from adresseerbare objecten refer to missing nummeraanduidingen, for example {}",
                bag_id(self.links[index].address, 16)
            );
        }
        Ok(())
    }
}

/// Rows a worker encoded, with their identificaties for the duplicate check.
struct Chunk {
    ids: Vec<i64>,
    encoded: Encoded,
}

/// Rows per chunk a worker encodes.
const CHUNK: usize = 1024;

struct Addresses<'a> {
    format: Format,
    places: &'a Resolver,
}

impl Converter for Addresses<'_> {
    type State = (Vec<i64>, Vec<Row>);
    type Item = Chunk;

    fn begin(&self) -> Self::State {
        (Vec::new(), Vec::new())
    }

    fn convert(
        &self,
        (ids, rows): &mut Self::State,
        object: BagObject,
        emit: &mut dyn FnMut(Chunk) -> Result<()>,
    ) -> Result<()> {
        let BagObject::Address(address) = object else {
            unreachable!("panden are not selected");
        };
        let id = address.id;
        if let Some(row) = self.places.resolve(address)? {
            ids.push(id);
            rows.push(row);
            if rows.len() == CHUNK {
                emit(Chunk {
                    ids: std::mem::take(ids),
                    encoded: self.format.encode(&ADDRESS_LIST, std::mem::take(rows)),
                })?;
            }
        }
        Ok(())
    }

    fn end(
        &self,
        (ids, rows): Self::State,
        emit: &mut dyn FnMut(Chunk) -> Result<()>,
    ) -> Result<()> {
        emit(Chunk {
            ids,
            encoded: self.format.encode(&ADDRESS_LIST, rows),
        })
    }
}
