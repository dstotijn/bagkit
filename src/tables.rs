//! The `tables` shape: one dataset per object type, as the SQLite core schema has them.
//!
//! Rows are written as they arrive. Import rules are checked in memory: duplicate
//! identificaties, a nummeraanduiding claimed by two adresseerbare objecten or none claimed
//! at all, adresseerbare objecten without exactly one hoofdadres, and references between
//! object types that were both read.
use crate::{
    dataset::{
        ADDRESSABLE_OBJECT_BUILDINGS, ADDRESSABLE_OBJECTS, ADDRESSES, BUILDINGS, Encoded, Extents,
        Format, LOCALITIES, MUNICIPALITIES, PUBLIC_SPACES, Row, Table, Value,
    },
    derived::object_point,
    extract::{Converter, Extract, ObjectType, Part, ReadStats, Selection},
    geometry::envelope,
    ids::{self, Ids, Sorted},
    municipalities,
    output::Writer,
    xml::{Address, BagObject, LocalityMunicipality, bag_id, check_main_address, object_type},
};
use anyhow::{Result, bail, ensure};
use std::{
    collections::{BTreeSet, HashMap},
    sync::atomic::{AtomicBool, Ordering},
};

/// The tables a selection writes. Link tables need both sides.
pub fn tables(selection: Selection) -> Vec<&'static Table> {
    let mut tables = Vec::new();
    for (object_type, table) in [
        (ObjectType::Locality, &LOCALITIES),
        (ObjectType::MunicipalityRelation, &MUNICIPALITIES),
        (ObjectType::PublicSpace, &PUBLIC_SPACES),
        (ObjectType::Address, &ADDRESSES),
        (ObjectType::AddressableObject, &ADDRESSABLE_OBJECTS),
        (ObjectType::Building, &BUILDINGS),
    ] {
        if selection.contains(object_type) {
            tables.push(table);
        }
    }
    if selection.contains(ObjectType::AddressableObject) && selection.contains(ObjectType::Building)
    {
        tables.push(&ADDRESSABLE_OBJECT_BUILDINGS);
    }
    tables
}

/// What a tables conversion wrote, beyond the rows themselves.
#[derive(Debug, Default)]
pub struct Written {
    pub read: ReadStats,
    /// Adresseerbare objecten whose punt lies outside RD New's area, and so have no WGS84
    /// coordinates.
    pub points_outside_rd_new: u64,
    /// Geometrie extents per table, `[min_x, min_y, max_x, max_y]`, when geometrie was read.
    pub extents: Extents,
    /// Gemeentecodes in use, including ones bagkit has no name for.
    pub municipalities: BTreeSet<i64>,
}

/// Reads `selection` from `extract` and writes its tables to `writer`.
pub fn write(extract: &Extract, selection: Selection, writer: &mut Writer) -> Result<Written> {
    let format = writer.format();
    let links_wanted = selection.contains(ObjectType::Address)
        && selection.contains(ObjectType::AddressableObject);
    let mut first = First::default();
    let converter = FirstConverter {
        format,
        footprints: extract.footprints,
        selection,
    };
    let mut read = extract.read(Part::First, selection, &converter, &mut |item| {
        first.add(item, writer)
    })?;
    let points_outside_rd_new = first.points_outside_rd_new;
    let object_extent = first.object_extent;
    let early = std::mem::take(&mut first.early);
    let (state, first_part_buildings) = first.finish(selection, links_wanted, writer)?;

    let mut second = Second {
        building_ids: first_part_buildings,
        ..Second::default()
    };
    let converter = SecondConverter {
        format,
        state: &state,
        selection,
    };
    // Nummeraanduidingen from first-part documents, now that their objects are known.
    let rows = early
        .into_iter()
        .map(|address| {
            let mut ids = Vec::new();
            let row = converter.address(&mut ids, address)?;
            second.address_ids.extend(ids);
            Ok(row)
        })
        .collect::<Result<Vec<_>>>()?;
    for chunk in rows.chunks(CHUNK) {
        writer.write(format.encode(&ADDRESSES, chunk.to_vec()))?;
    }
    read.add(
        extract.read(Part::Second, selection, &converter, &mut |item| {
            second.add(item, writer)
        })?,
    );
    ensure!(
        read.documents > 0 && read.records > 0,
        "no BAG records found in the supplied inputs"
    );
    writer.finish()?;
    let mut building_extent = second.building_extent;
    building_extent.merge(state.first_part_building_extent);
    second.finish(&state, selection, links_wanted)?;
    let mut extents = Vec::new();
    if extract.footprints {
        if selection.contains(ObjectType::Building) {
            extents.push((BUILDINGS.name, building_extent.bounds()));
        }
        if selection.contains(ObjectType::AddressableObject) {
            extents.push((ADDRESSABLE_OBJECTS.name, object_extent.bounds()));
        }
    }
    Ok(Written {
        read,
        points_outside_rd_new,
        extents,
        municipalities: state.municipalities.values().copied().collect(),
    })
}

/// Rows per chunk a worker encodes.
const CHUNK: usize = 1024;

/// Rows of several tables, gathered per document and encoded in chunks.
struct Chunks {
    format: Format,
    rows: Vec<(&'static Table, Vec<Row>)>,
}

impl Chunks {
    fn new(format: Format) -> Self {
        Self {
            format,
            rows: Vec::new(),
        }
    }

    /// Adds a row; returns a full chunk to emit.
    fn push(&mut self, table: &'static Table, row: Row) -> Option<Encoded> {
        let index = match self.rows.iter().position(|(t, _)| std::ptr::eq(*t, table)) {
            Some(index) => index,
            None => {
                self.rows.push((table, Vec::with_capacity(CHUNK)));
                self.rows.len() - 1
            }
        };
        let rows = &mut self.rows[index].1;
        rows.push(row);
        (rows.len() == CHUNK).then(|| self.format.encode(table, std::mem::take(rows)))
    }

    fn drain(&mut self) -> impl Iterator<Item = Encoded> + '_ {
        let format = self.format;
        self.rows
            .drain(..)
            .filter(|(_, rows)| !rows.is_empty())
            .map(move |(table, rows)| format.encode(table, rows))
    }
}

/// An extent being grown.
#[derive(Debug, Clone, Copy)]
struct Extent([f64; 4]);

impl Default for Extent {
    fn default() -> Self {
        Self([
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ])
    }
}

impl Extent {
    fn add(&mut self, [min_x, min_y, max_x, max_y]: [f64; 4]) {
        self.0[0] = self.0[0].min(min_x);
        self.0[1] = self.0[1].min(min_y);
        self.0[2] = self.0[2].max(max_x);
        self.0[3] = self.0[3].max(max_y);
    }

    fn merge(&mut self, other: Extent) {
        self.add(other.0);
    }

    fn bounds(self) -> Option<[f64; 4]> {
        (self.0[0] <= self.0[2]).then_some(self.0)
    }
}

enum FirstItem {
    Rows(Encoded),
    Locality(crate::xml::Locality),
    Relation(LocalityMunicipality),
    PublicSpace {
        id: i64,
        locality: i64,
    },
    Objects(Objects),
    /// A nummeraanduiding in a first-part document, resolved after the first part.
    Address(Address),
    /// A pand in a first-part document.
    Buildings(Buildings),
}

/// What the writer keeps of a chunk of adresseerbare objecten.
#[derive(Default)]
struct Objects {
    ids: Vec<i64>,
    /// Nummeraanduiding and adresseerbaar object, with [`MAIN_ADDRESS`] set for hoofdadressen.
    links: Vec<(i64, i64)>,
    building_refs: Vec<i64>,
    outside: u64,
    extent: Extent,
}

#[derive(Default)]
struct Buildings {
    ids: Vec<i64>,
    extent: Extent,
}

/// Marks a hoofdadres in [`Objects::links`]; identificaties are below 2^51.
const MAIN_ADDRESS: i64 = 1 << 62;

struct FirstConverter {
    format: Format,
    footprints: bool,
    selection: Selection,
}

struct FirstState {
    chunks: Chunks,
    objects: Objects,
    buildings: Buildings,
}

impl Converter for FirstConverter {
    type State = FirstState;
    type Item = FirstItem;

    fn begin(&self) -> FirstState {
        FirstState {
            chunks: Chunks::new(self.format),
            objects: Objects::default(),
            buildings: Buildings::default(),
        }
    }

    fn convert(
        &self,
        state: &mut FirstState,
        object: BagObject,
        emit: &mut dyn FnMut(FirstItem) -> Result<()>,
    ) -> Result<()> {
        let full = match object {
            BagObject::Locality(l) => return emit(FirstItem::Locality(l)),
            BagObject::LocalityMunicipality(m) => return emit(FirstItem::Relation(m)),
            BagObject::Address(a) => return emit(FirstItem::Address(a)),
            BagObject::PublicSpace(p) => {
                emit(FirstItem::PublicSpace {
                    id: p.id,
                    locality: p.locality_id,
                })?;
                state.chunks.push(&PUBLIC_SPACES, public_space_row(p))
            }
            BagObject::AddressableObject(o) => {
                check_main_address(&o)?;
                let (point, wgs84) = object_point(o.geometry.as_ref());
                let objects = &mut state.objects;
                objects.outside += u64::from(point.is_some() && wgs84.is_none());
                // Every object has a hoofdadres, so a repeated object repeats a link,
                // which finds it; without nummeraanduidingen, identificaties are kept instead.
                if self.selection.contains(ObjectType::Address) {
                    for &(address, main) in &o.addresses {
                        objects
                            .links
                            .push((address, o.id | if main { MAIN_ADDRESS } else { 0 }));
                    }
                } else {
                    objects.ids.push(o.id);
                }
                let footprint = match &o.geometry {
                    Some(crate::xml::Geometry::Polygon(rings)) if self.footprints => {
                        objects.extent.add(envelope(&rings[0]));
                        self.format.polygon(rings)
                    }
                    _ => Value::Null,
                };
                if self.selection.contains(ObjectType::Building) {
                    for &building in &o.building_ids {
                        objects.building_refs.push(building);
                        if let Some(encoded) = state.chunks.push(
                            &ADDRESSABLE_OBJECT_BUILDINGS,
                            vec![Value::Integer(o.id), Value::Integer(building)],
                        ) {
                            emit(FirstItem::Rows(encoded))?;
                        }
                    }
                }
                let row = vec![
                    Value::Integer(o.id),
                    Value::Text(o.kind.bag_name().into()),
                    Value::Text(o.status),
                    Value::Text(serde_json::to_string(&o.usage_purposes)?),
                    Value::integer(o.usable_area_m2),
                    footprint,
                    Value::real(point.map(|p| p[0])),
                    Value::real(point.map(|p| p[1])),
                    Value::real(wgs84.map(|p| p[0])),
                    Value::real(wgs84.map(|p| p[1])),
                ];
                let full = state.chunks.push(&ADDRESSABLE_OBJECTS, row);
                if full.is_some() {
                    emit(FirstItem::Objects(std::mem::take(&mut state.objects)))?;
                }
                full
            }
            BagObject::Building(b) => {
                let row = building_row(self.format, b, &mut state.buildings);
                state.chunks.push(&BUILDINGS, row)
            }
        };
        if let Some(encoded) = full {
            emit(FirstItem::Rows(encoded))?;
        }
        Ok(())
    }

    fn end(
        &self,
        mut state: FirstState,
        emit: &mut dyn FnMut(FirstItem) -> Result<()>,
    ) -> Result<()> {
        for encoded in state.chunks.drain().collect::<Vec<_>>() {
            emit(FirstItem::Rows(encoded))?;
        }
        emit(FirstItem::Objects(state.objects))?;
        emit(FirstItem::Buildings(state.buildings))
    }
}

fn public_space_row(p: crate::xml::PublicSpace) -> Row {
    vec![
        Value::Integer(p.id),
        Value::Text(p.name),
        Value::text(p.short_name),
        Value::Text(p.space_type),
        Value::Text(p.status),
        Value::Integer(p.locality_id),
    ]
}

fn building_row(format: Format, b: crate::xml::Building, buildings: &mut Buildings) -> Row {
    buildings.ids.push(b.id);
    let footprint = match &b.footprint {
        Some(rings) => {
            buildings.extent.add(envelope(&rings[0]));
            format.polygon(rings)
        }
        None => Value::Null,
    };
    vec![
        Value::Integer(b.id),
        Value::Integer(b.construction_year),
        Value::Text(b.status),
        footprint,
    ]
}

/// The first part's state, collected by the writer.
#[derive(Default)]
struct First {
    localities: Vec<crate::xml::Locality>,
    relations: Vec<LocalityMunicipality>,
    public_spaces: HashMap<i64, i64>,
    object_ids: Ids,
    links: Vec<(i64, i64)>,
    building_refs: Vec<i64>,
    building_ids: Ids,
    early: Vec<Address>,
    points_outside_rd_new: u64,
    object_extent: Extent,
    building_extent: Extent,
}

impl First {
    fn add(&mut self, item: FirstItem, writer: &mut Writer) -> Result<()> {
        match item {
            FirstItem::Rows(encoded) => writer.write(encoded)?,
            FirstItem::Locality(l) => self.localities.push(l),
            FirstItem::Relation(r) => self.relations.push(r),
            FirstItem::PublicSpace { id, locality } => {
                ensure!(
                    self.public_spaces.insert(id, locality).is_none(),
                    "duplicate OpenbareRuimte identificatie {}",
                    bag_id(id, 16)
                );
            }
            FirstItem::Objects(o) => {
                self.object_ids.extend(o.ids);
                self.links.extend(o.links);
                self.building_refs.extend(o.building_refs);
                self.points_outside_rd_new += o.outside;
                self.object_extent.merge(o.extent);
            }
            FirstItem::Address(a) => self.early.push(a),
            FirstItem::Buildings(b) => {
                self.building_ids.extend(b.ids);
                self.building_extent.merge(b.extent);
            }
        }
        Ok(())
    }

    /// Writes woonplaatsen and gemeenten, checks what the first part read, and returns
    /// what nummeraanduidingen are resolved against, with the panden found so far.
    fn finish(
        mut self,
        selection: Selection,
        links_wanted: bool,
        writer: &mut Writer,
    ) -> Result<(State, Ids)> {
        let format = writer.format();
        let locality_ids: BTreeSet<i64> = {
            let mut ids = BTreeSet::new();
            for l in &self.localities {
                ensure!(
                    ids.insert(l.id),
                    "duplicate Woonplaats identificatie {}",
                    bag_id(l.id, 4)
                );
            }
            ids
        };
        let municipalities = if selection.contains(ObjectType::MunicipalityRelation)
            && selection.contains(ObjectType::Locality)
        {
            municipalities::assign(&self.relations, locality_ids.iter().copied())?
        } else {
            HashMap::new()
        };
        let rows = self.localities.drain(..).map(|l| {
            vec![
                Value::Integer(l.id),
                Value::Text(l.name),
                Value::Text(l.status),
                Value::boolean(l.is_observed),
                Value::integer(municipalities.get(&l.id).copied()),
            ]
        });
        for chunk in rows.collect::<Vec<_>>().chunks(CHUNK) {
            writer.write(format.encode(&LOCALITIES, chunk.to_vec()))?;
        }
        if selection.contains(ObjectType::MunicipalityRelation) {
            let mut rows: Vec<Row> = municipalities::all()
                .map(|m| {
                    vec![
                        Value::Integer(m.id),
                        Value::Text(m.name.into()),
                        Value::Text(m.province.into()),
                    ]
                })
                .collect();
            let known: BTreeSet<i64> = municipalities::all().map(|m| m.id).collect();
            let unnamed: BTreeSet<i64> = municipalities
                .values()
                .copied()
                .filter(|id| !known.contains(id))
                .collect();
            rows.extend(
                unnamed
                    .into_iter()
                    .map(|id| vec![Value::Integer(id), Value::Null, Value::Null]),
            );
            writer.write(format.encode(&MUNICIPALITIES, rows))?;
        }
        if selection.contains(ObjectType::Locality) {
            let missing: BTreeSet<i64> = self
                .public_spaces
                .values()
                .copied()
                .filter(|l| !locality_ids.contains(l))
                .collect();
            if let Some(first) = missing.first() {
                let count = self
                    .public_spaces
                    .values()
                    .filter(|l| missing.contains(l))
                    .count();
                bail!(
                    "{count} openbare ruimten refer to missing woonplaatsen, for example {}",
                    bag_id(*first, 4)
                );
            }
        }
        if let Err(id) = std::mem::take(&mut self.object_ids).sorted_unique() {
            bail!(
                "duplicate {} identificatie {}",
                object_type(id),
                bag_id(id, 16)
            );
        }
        self.links.sort_unstable();
        let (mut count, mut example) = (0u64, None);
        let object = |link: i64| link & !MAIN_ADDRESS;
        for pair in self.links.windows(2) {
            if pair[0].0 == pair[1].0 {
                // The same object listing a nummeraanduiding twice means the object repeats.
                if object(pair[0].1) == object(pair[1].1) {
                    let id = object(pair[0].1);
                    bail!(
                        "duplicate {} identificatie {}",
                        object_type(id),
                        bag_id(id, 16)
                    );
                }
                count += 1;
                example.get_or_insert(pair[0].0);
            }
        }
        ensure!(
            count == 0,
            "{count} nummeraanduidingen belong to more than one adresseerbaar object, for example {}",
            example.map(|id| bag_id(id, 16)).unwrap_or_default()
        );
        if !links_wanted {
            self.links = Vec::new();
        }
        let buildings = std::mem::take(&mut self.building_ids);
        let state = State {
            matched: self.links.iter().map(|_| AtomicBool::new(false)).collect(),
            links: self.links,
            public_spaces: self.public_spaces,
            localities: locality_ids,
            municipalities,
            building_refs: ids::distinct(self.building_refs),
            first_part_building_extent: self.building_extent,
        };
        Ok((state, buildings))
    }
}

/// What nummeraanduidingen are resolved and checked against; read-only during the second
/// part.
struct State {
    /// Nummeraanduiding and packed adresseerbaar object, sorted by nummeraanduiding.
    links: Vec<(i64, i64)>,
    matched: Vec<AtomicBool>,
    public_spaces: HashMap<i64, i64>,
    localities: BTreeSet<i64>,
    municipalities: HashMap<i64, i64>,
    /// Panden that verblijfsobjecten refer to.
    building_refs: Sorted,
    first_part_building_extent: Extent,
}

enum SecondItem {
    Rows(Encoded),
    AddressIds(Vec<i64>),
    Buildings(Buildings),
}

struct SecondConverter<'a> {
    format: Format,
    state: &'a State,
    selection: Selection,
}

struct SecondState {
    chunks: Chunks,
    address_ids: Vec<i64>,
    buildings: Buildings,
}

impl SecondConverter<'_> {
    /// Resolves a nummeraanduiding's adresseerbaar object and checks its references.
    fn address(&self, address_ids: &mut Vec<i64>, a: Address) -> Result<Row> {
        let state = self.state;
        let link = state
            .links
            .binary_search_by_key(&a.id, |l| l.0)
            .ok()
            .map(|index| {
                state.matched[index].store(true, Ordering::Relaxed);
                state.links[index].1
            });
        if self.selection.contains(ObjectType::PublicSpace)
            && !state.public_spaces.contains_key(&a.public_space_id)
        {
            bail!(
                "nummeraanduidingen refer to missing openbare ruimten, for example {}",
                bag_id(a.public_space_id, 16)
            );
        }
        if self.selection.contains(ObjectType::Locality)
            && let Some(locality) = a.locality_id
            && !state.localities.contains(&locality)
        {
            bail!(
                "nummeraanduidingen refer to missing woonplaatsen, for example {}",
                bag_id(locality, 4)
            );
        }
        let row = vec![
            Value::Integer(a.id),
            Value::text(a.postcode),
            Value::Integer(a.house_number),
            Value::text(a.house_letter),
            Value::text(a.house_number_addition),
            Value::Text(a.status),
            Value::Integer(a.public_space_id),
            Value::integer(a.locality_id),
            Value::integer(link.map(|l| l & !MAIN_ADDRESS)),
            match link {
                Some(l) => Value::boolean(l & MAIN_ADDRESS == 0),
                None => Value::Null,
            },
        ];
        address_ids.push(a.id);
        Ok(row)
    }
}

impl Converter for SecondConverter<'_> {
    type State = SecondState;
    type Item = SecondItem;

    fn begin(&self) -> SecondState {
        SecondState {
            chunks: Chunks::new(self.format),
            address_ids: Vec::new(),
            buildings: Buildings::default(),
        }
    }

    fn convert(
        &self,
        state: &mut SecondState,
        object: BagObject,
        emit: &mut dyn FnMut(SecondItem) -> Result<()>,
    ) -> Result<()> {
        let full = match object {
            BagObject::Address(a) => {
                let row = self.address(&mut state.address_ids, a)?;
                state.chunks.push(&ADDRESSES, row)
            }
            BagObject::Building(b) => {
                let row = building_row(self.format, b, &mut state.buildings);
                state.chunks.push(&BUILDINGS, row)
            }
            _ => unreachable!("the second part reads only nummeraanduidingen and panden"),
        };
        if let Some(encoded) = full {
            emit(SecondItem::Rows(encoded))?;
        }
        Ok(())
    }

    fn end(
        &self,
        mut state: SecondState,
        emit: &mut dyn FnMut(SecondItem) -> Result<()>,
    ) -> Result<()> {
        for encoded in state.chunks.drain().collect::<Vec<_>>() {
            emit(SecondItem::Rows(encoded))?;
        }
        emit(SecondItem::AddressIds(state.address_ids))?;
        emit(SecondItem::Buildings(state.buildings))
    }
}

#[derive(Default)]
struct Second {
    address_ids: Ids,
    building_ids: Ids,
    building_extent: Extent,
}

impl Second {
    fn add(&mut self, item: SecondItem, writer: &mut Writer) -> Result<()> {
        match item {
            SecondItem::Rows(encoded) => writer.write(encoded)?,
            SecondItem::AddressIds(ids) => self.address_ids.extend(ids),
            SecondItem::Buildings(b) => {
                self.building_ids.extend(b.ids);
                self.building_extent.merge(b.extent);
            }
        }
        Ok(())
    }

    /// Checks identificaties and links once everything is read.
    fn finish(self, state: &State, selection: Selection, links_wanted: bool) -> Result<()> {
        if let Err(id) = self.address_ids.sorted_unique() {
            bail!(
                "duplicate Nummeraanduiding identificatie {}",
                bag_id(id, 16)
            );
        }
        let buildings = match self.building_ids.sorted_unique() {
            Ok(buildings) => buildings,
            Err(id) => bail!("duplicate Pand identificatie {}", bag_id(id, 16)),
        };
        if links_wanted {
            let unmatched: Vec<usize> = state
                .matched
                .iter()
                .enumerate()
                .filter(|(_, m)| !m.load(Ordering::Relaxed))
                .map(|(index, _)| index)
                .collect();
            if let Some(&first) = unmatched.first() {
                bail!(
                    "{} links from adresseerbare objecten refer to missing nummeraanduidingen, for example {}",
                    unmatched.len(),
                    bag_id(state.links[first].0, 16)
                );
            }
        }
        if selection.contains(ObjectType::Building)
            && selection.contains(ObjectType::AddressableObject)
        {
            let missing: Vec<i64> = buildings.missing(&state.building_refs).collect();
            if let Some(first) = missing.first() {
                bail!(
                    "{} links from verblijfsobjecten refer to missing panden, for example {}",
                    missing.len(),
                    bag_id(*first, 16)
                );
            }
        }
        Ok(())
    }
}
