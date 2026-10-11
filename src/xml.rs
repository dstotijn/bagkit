use crate::geometry::{Ring, parse_positions};
use anyhow::{Context, Result, bail, ensure};
use quick_xml::{Reader, events::Event};
use std::io::BufRead;

/// The BAG object types bagkit reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Woonplaats,
    OpenbareRuimte,
    Nummeraanduiding,
    Verblijfsobject,
    Ligplaats,
    Standplaats,
    Pand,
    /// Kadaster's relation between a woonplaats and its gemeente, supplied with the extract
    /// but not part of the BAG itself.
    GemeenteWoonplaatsRelatie,
}

impl Kind {
    fn from_tag(name: &str) -> Option<Self> {
        Some(match name {
            "Woonplaats" => Self::Woonplaats,
            "OpenbareRuimte" => Self::OpenbareRuimte,
            "Nummeraanduiding" => Self::Nummeraanduiding,
            "Verblijfsobject" => Self::Verblijfsobject,
            "Ligplaats" => Self::Ligplaats,
            "Standplaats" => Self::Standplaats,
            "Pand" => Self::Pand,
            "GemeenteWoonplaatsRelatie" => Self::GemeenteWoonplaatsRelatie,
            _ => return None,
        })
    }
}

impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

/// When a voorkomen is valid, and whether the BAG still considers it part of the
/// registration.
#[derive(Debug, Default, Clone)]
pub struct Validity {
    pub begin: String,
    pub end: String,
    pub has_history: bool,
    /// Inactief (`tijdstipInactief`) or niet-BAG (`tijdstipNietBagLV`).
    pub inactive: bool,
}

impl Validity {
    /// Whether the voorkomen describes its object on `date`: `begin <= date < end`.
    pub fn is_current(&self, date: &str) -> Result<bool> {
        if self.inactive {
            return Ok(false);
        }
        if !self.has_history {
            return Ok(true);
        }
        ensure!(!self.begin.is_empty(), "record has no beginGeldigheid");
        validate_date(&self.begin)?;
        if !self.end.is_empty() {
            validate_date(&self.end)?;
        }
        Ok(self.begin.as_str() <= date && (self.end.is_empty() || date < self.end.as_str()))
    }
}

/// One BAG voorkomen as read from XML. Convert it with [`Record::into_object`] after
/// selecting it, so incomplete historical voorkomens never need to be valid.
#[derive(Debug)]
pub struct Record {
    pub kind: Kind,
    pub id: String,
    pub validity: Validity,
    fields: Fields,
}

#[derive(Debug, Default)]
struct Fields {
    name: String,
    short_name: String,
    status: String,
    object_type: String,
    observed: bool,
    locality: String,
    municipality: String,
    public_space: String,
    house_number: Option<i64>,
    letter: String,
    addition: String,
    postcode: String,
    area: Option<i64>,
    year: Option<i64>,
    uses: Vec<String>,
    addresses: Vec<(String, bool)>,
    buildings: Vec<String>,
    point: String,
    point_dimension: Option<usize>,
    polygon_dimension: Option<usize>,
    rings: Vec<RawRing>,
}

#[derive(Debug)]
struct RawRing {
    text: String,
    count: Option<usize>,
    dimension: Option<usize>,
}

/// The type of an adresseerbaar object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    /// A verblijfsobject.
    Unit,
    /// A ligplaats.
    Mooring,
    /// A standplaats.
    Pitch,
}

impl ObjectKind {
    pub fn bag_name(self) -> &'static str {
        match self {
            Self::Unit => "Verblijfsobject",
            Self::Mooring => "Ligplaats",
            Self::Pitch => "Standplaats",
        }
    }
}

/// The geometrie of an adresseerbaar object.
#[derive(Debug, Clone, PartialEq)]
pub enum Geometry {
    /// A punt, in RD New.
    Point([f64; 2]),
    /// A vlak: exterior ring first, then interior rings, in RD New.
    Polygon(Vec<Ring>),
}

/// A woonplaats.
#[derive(Debug)]
pub struct Locality {
    /// The identificatie: the woonplaatscode.
    pub id: i64,
    /// The naam.
    pub name: String,
    pub status: String,
    /// Geconstateerd.
    pub is_observed: bool,
}

/// The gemeente a woonplaats belongs to (GEM-WPL-RELATIE).
#[derive(Debug)]
pub struct LocalityMunicipality {
    /// The woonplaats identificatie.
    pub locality_id: i64,
    /// The gemeentecode.
    pub municipality_id: i64,
}

/// An openbare ruimte.
#[derive(Debug)]
pub struct PublicSpace {
    /// The identificatie.
    pub id: i64,
    /// The naam.
    pub name: String,
    /// The verkorte naam (NEN 5825).
    pub short_name: Option<String>,
    /// The type, such as `Weg`.
    pub space_type: String,
    pub status: String,
    /// The identificatie of the woonplaats it ligt in.
    pub locality_id: i64,
}

/// A nummeraanduiding.
#[derive(Debug)]
pub struct Address {
    /// The identificatie.
    pub id: i64,
    pub postcode: Option<String>,
    /// The huisnummer.
    pub house_number: i64,
    /// The huisletter.
    pub house_letter: Option<String>,
    /// The huisnummertoevoeging.
    pub house_number_addition: Option<String>,
    pub status: String,
    /// The identificatie of the openbare ruimte it ligt aan.
    pub public_space_id: i64,
    /// The woonplaats it ligt in, when it names its own rather than its openbare ruimte's.
    pub locality_id: Option<i64>,
}

/// An adresseerbaar object: a verblijfsobject, ligplaats or standplaats.
#[derive(Debug)]
pub struct AddressableObject {
    /// The identificatie.
    pub id: i64,
    /// The type.
    pub kind: ObjectKind,
    pub status: String,
    /// The gebruiksdoelen, sorted and deduplicated.
    pub usage_purposes: Vec<String>,
    /// The oppervlakte in square metres, for verblijfsobjecten.
    pub usable_area_m2: Option<i64>,
    /// The geometrie: a punt or a vlak.
    pub geometry: Option<Geometry>,
    /// Nummeraanduiding identificaties with whether each is the hoofdadres, sorted and
    /// deduplicated.
    pub addresses: Vec<(i64, bool)>,
    /// The identificaties of the panden it maakt deel uit van, sorted and deduplicated.
    pub building_ids: Vec<i64>,
}

/// A pand.
#[derive(Debug)]
pub struct Building {
    /// The identificatie.
    pub id: i64,
    /// The oorspronkelijk bouwjaar.
    pub construction_year: i64,
    pub status: String,
    /// The geometrie, a vlak; `None` when geometrie is not read.
    pub footprint: Option<Vec<Ring>>,
}

/// A current voorkomen, typed by object type.
#[derive(Debug)]
pub enum BagObject {
    Locality(Locality),
    LocalityMunicipality(LocalityMunicipality),
    PublicSpace(PublicSpace),
    Address(Address),
    AddressableObject(AddressableObject),
    Building(Building),
}

fn optional(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn required(value: String, kind: Kind, field: &str) -> Result<String> {
    ensure!(!value.is_empty(), "{kind} has no {field}");
    Ok(value)
}

/// Fails unless an adresseerbaar object has exactly one hoofdadres.
pub fn check_main_address(o: &AddressableObject) -> Result<()> {
    let main_addresses = o.addresses.iter().filter(|(_, main)| *main).count();
    ensure!(
        main_addresses == 1,
        "{} {} has {main_addresses} hoofdadressen",
        o.kind.bag_name(),
        bag_id(o.id, 16)
    );
    Ok(())
}

/// The BAG object type of an adresseerbaar object identificatie, from its object type code
/// (digits 5 and 6).
pub fn object_type(id: i64) -> &'static str {
    match id / 10_000_000_000 % 100 {
        1 => "Verblijfsobject",
        2 => "Ligplaats",
        3 => "Standplaats",
        _ => "addressable object",
    }
}

/// Formats an identificatie in its official zero-padded form.
pub fn bag_id(id: i64, digits: usize) -> String {
    format!("{id:0digits$}")
}

/// Parses an identificatie of exactly `digits` digits.
pub fn parse_id(value: &str, digits: usize) -> Result<i64> {
    ensure!(
        value.len() == digits && value.bytes().all(|b| b.is_ascii_digit()),
        "identificatie '{value}' is not {digits} digits"
    );
    Ok(value.parse()?)
}

impl Record {
    /// Converts a selected voorkomen into a typed object. The geometrie of panden is parsed
    /// only when `footprints` is set; that of adresseerbare objecten always is.
    pub fn into_object(self, footprints: bool) -> Result<BagObject> {
        let Record {
            kind, id, fields, ..
        } = self;
        Self::convert(kind, &id, fields, footprints).with_context(|| format!("{kind} {id}"))
    }

    fn convert(kind: Kind, id: &str, f: Fields, footprints: bool) -> Result<BagObject> {
        let four_digits = matches!(kind, Kind::Woonplaats | Kind::GemeenteWoonplaatsRelatie);
        let id = parse_id(id, if four_digits { 4 } else { 16 })?;
        let status = required(f.status, kind, "status")?;
        Ok(match kind {
            Kind::Woonplaats => BagObject::Locality(Locality {
                id,
                name: required(f.name, kind, "naam")?,
                status,
                is_observed: f.observed,
            }),
            Kind::GemeenteWoonplaatsRelatie => {
                BagObject::LocalityMunicipality(LocalityMunicipality {
                    locality_id: id,
                    municipality_id: parse_id(
                        &required(f.municipality, kind, "gerelateerdeGemeente")?,
                        4,
                    )?,
                })
            }
            Kind::OpenbareRuimte => BagObject::PublicSpace(PublicSpace {
                id,
                name: required(f.name, kind, "naam")?,
                short_name: optional(f.short_name),
                space_type: required(f.object_type, kind, "type")?,
                status,
                locality_id: parse_id(&required(f.locality, kind, "WoonplaatsRef")?, 4)?,
            }),
            Kind::Nummeraanduiding => BagObject::Address(Address {
                id,
                postcode: optional(f.postcode),
                house_number: f
                    .house_number
                    .context("Nummeraanduiding has no huisnummer")?,
                house_letter: optional(f.letter),
                house_number_addition: optional(f.addition),
                status,
                public_space_id: parse_id(
                    &required(f.public_space, kind, "OpenbareRuimteRef")?,
                    16,
                )?,
                locality_id: optional(f.locality).map(|r| parse_id(&r, 4)).transpose()?,
            }),
            Kind::Verblijfsobject | Kind::Ligplaats | Kind::Standplaats => {
                let geometry = if !f.point.is_empty() {
                    let numbers = f.point.split_ascii_whitespace().count();
                    let point = parse_positions(&f.point, f.point_dimension.unwrap_or(numbers))?;
                    ensure!(
                        point.len() == 1,
                        "point geometry has {} positions",
                        point.len()
                    );
                    Some(Geometry::Point(point[0]))
                } else {
                    polygon(&f.rings, f.polygon_dimension)?.map(Geometry::Polygon)
                };
                let mut usage_purposes = f.uses;
                usage_purposes.sort_unstable();
                usage_purposes.dedup();
                let mut addresses = f
                    .addresses
                    .iter()
                    .map(|(r, main)| Ok((parse_id(r, 16)?, *main)))
                    .collect::<Result<Vec<_>>>()?;
                // A hoofdadres link sorts after a nevenadres link to the same nummeraanduiding
                // and wins.
                addresses.sort_unstable();
                addresses.dedup_by(|next, kept| {
                    next.0 == kept.0 && {
                        kept.1 |= next.1;
                        true
                    }
                });
                let mut building_ids = f
                    .buildings
                    .iter()
                    .map(|r| parse_id(r, 16))
                    .collect::<Result<Vec<_>>>()?;
                building_ids.sort_unstable();
                building_ids.dedup();
                BagObject::AddressableObject(AddressableObject {
                    id,
                    kind: match kind {
                        Kind::Verblijfsobject => ObjectKind::Unit,
                        Kind::Ligplaats => ObjectKind::Mooring,
                        _ => ObjectKind::Pitch,
                    },
                    status,
                    usage_purposes,
                    usable_area_m2: f.area,
                    geometry,
                    addresses,
                    building_ids,
                })
            }
            Kind::Pand => BagObject::Building(Building {
                id,
                construction_year: f.year.context("Pand has no oorspronkelijkBouwjaar")?,
                status,
                footprint: if footprints {
                    polygon(&f.rings, f.polygon_dimension)?
                } else {
                    None
                },
            }),
        })
    }

    fn field(&mut self, path: &[&str], value: &str) -> Result<()> {
        let value = value.trim();
        let leaf = path.last().copied().unwrap_or("");
        let direct = path.len() == 2;
        let f = &mut self.fields;
        match leaf {
            "identificatie" if path.get(1) == Some(&"gerelateerdeWoonplaats") => {
                self.id = value.into()
            }
            "identificatie" if path.get(1) == Some(&"gerelateerdeGemeente") => {
                f.municipality = value.into()
            }
            "identificatie" if direct => self.id = value.into(),
            "naam" if direct => f.name = value.into(),
            "status" if direct => f.status = value.into(),
            "type" if direct => f.object_type = value.into(),
            "geconstateerd" if direct => f.observed = value == "J",
            "huisnummer" if direct => f.house_number = Some(value.parse()?),
            "huisletter" if direct => f.letter = value.into(),
            "huisnummertoevoeging" if direct => f.addition = value.into(),
            "postcode" if direct => f.postcode = value.into(),
            "oppervlakte" if direct => f.area = Some(value.parse()?),
            "oorspronkelijkBouwjaar" if direct => f.year = Some(value.parse()?),
            "gebruiksdoel" if direct => f.uses.push(value.into()),
            "verkorteNaam" if path.len() > 2 => f.short_name = value.into(),
            "beginGeldigheid" | "begindatumTijdvakGeldigheid" => self.validity.begin = value.into(),
            "eindGeldigheid" | "einddatumTijdvakGeldigheid" => self.validity.end = value.into(),
            "tijdstipInactief" | "tijdstipInactiefLV" | "tijdstipNietBagLV" => {
                self.validity.inactive = true
            }
            "WoonplaatsRef" => f.locality = value.into(),
            "OpenbareRuimteRef" => f.public_space = value.into(),
            "PandRef" => f.buildings.push(value.into()),
            "NummeraanduidingRef" => f
                .addresses
                .push((value.into(), path.contains(&"heeftAlsHoofdadres"))),
            "pos" if path.contains(&"geometrie") => f.point = value.into(),
            _ => (),
        }
        Ok(())
    }
}

/// Parses polygon rings. `count` (points per ring) decides the stride when present, since
/// it is exact; otherwise the declared dimension is used.
fn polygon(rings: &[RawRing], dimension: Option<usize>) -> Result<Option<Vec<Ring>>> {
    if rings.is_empty() {
        return Ok(None);
    }
    rings
        .iter()
        .map(|ring| {
            let numbers = ring.text.split_ascii_whitespace().count();
            let dimension = match ring.count {
                Some(count) if count > 0 && numbers % count == 0 => numbers / count,
                _ => ring.dimension.or(dimension).unwrap_or(2),
            };
            let points = parse_positions(&ring.text, dimension)?;
            ensure!(
                points.len() >= 4,
                "polygon ring has {} positions",
                points.len()
            );
            Ok(points)
        })
        .collect::<Result<_>>()
        .map(Some)
}

pub fn validate_date(date: &str) -> Result<()> {
    let bytes = date.as_bytes();
    ensure!(
        bytes.len() == 10
            && bytes[4] == b'-'
            && bytes[7] == b'-'
            && bytes
                .iter()
                .enumerate()
                .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit()),
        "expected a date in YYYY-MM-DD format"
    );
    let year = date[..4].parse()?;
    let month = date[5..7].parse()?;
    let day = date[8..].parse()?;
    ensure!(
        chrono::NaiveDate::from_ymd_opt(year, month, day).is_some(),
        "invalid date {date}"
    );
    Ok(())
}

fn local(name: &[u8]) -> &[u8] {
    name.rsplit(|b| *b == b':').next().unwrap_or(name)
}

// Intern known names so traversing millions of elements allocates no name strings. Other
// names become "", and their text is never read.
fn tag(name: &[u8]) -> &'static str {
    match name {
        b"bagObject" => "bagObject",
        b"Woonplaats" => "Woonplaats",
        b"OpenbareRuimte" => "OpenbareRuimte",
        b"Nummeraanduiding" => "Nummeraanduiding",
        b"Verblijfsobject" => "Verblijfsobject",
        b"Ligplaats" => "Ligplaats",
        b"Standplaats" => "Standplaats",
        b"Pand" => "Pand",
        b"GemeenteWoonplaatsRelatie" => "GemeenteWoonplaatsRelatie",
        b"GemeenteWoonplaatsRelatieProduct" => "GemeenteWoonplaatsRelatieProduct",
        b"gerelateerdeWoonplaats" => "gerelateerdeWoonplaats",
        b"gerelateerdeGemeente" => "gerelateerdeGemeente",
        b"begindatumTijdvakGeldigheid" => "begindatumTijdvakGeldigheid",
        b"einddatumTijdvakGeldigheid" => "einddatumTijdvakGeldigheid",
        b"Voorkomen" => "Voorkomen",
        b"Point" => "Point",
        b"Polygon" => "Polygon",
        b"posList" => "posList",
        b"identificatie" => "identificatie",
        b"naam" => "naam",
        b"status" => "status",
        b"type" => "type",
        b"geconstateerd" => "geconstateerd",
        b"huisnummer" => "huisnummer",
        b"huisletter" => "huisletter",
        b"huisnummertoevoeging" => "huisnummertoevoeging",
        b"postcode" => "postcode",
        b"oppervlakte" => "oppervlakte",
        b"oorspronkelijkBouwjaar" => "oorspronkelijkBouwjaar",
        b"gebruiksdoel" => "gebruiksdoel",
        b"verkorteNaam" => "verkorteNaam",
        b"beginGeldigheid" => "beginGeldigheid",
        b"eindGeldigheid" => "eindGeldigheid",
        b"tijdstipInactief" => "tijdstipInactief",
        b"tijdstipInactiefLV" => "tijdstipInactiefLV",
        b"tijdstipNietBagLV" => "tijdstipNietBagLV",
        b"WoonplaatsRef" => "WoonplaatsRef",
        b"OpenbareRuimteRef" => "OpenbareRuimteRef",
        b"PandRef" => "PandRef",
        b"NummeraanduidingRef" => "NummeraanduidingRef",
        b"pos" => "pos",
        b"geometrie" => "geometrie",
        b"exterior" => "exterior",
        b"interior" => "interior",
        b"heeftAlsHoofdadres" => "heeftAlsHoofdadres",
        b"StandTechnischeDatum" => "StandTechnischeDatum",
        _ => "",
    }
}

/// The XML the voorkomen parser reads. Empty elements yield a start and an end; comments,
/// processing instructions and declarations are skipped.
enum Token<'a> {
    Start {
        name: &'a [u8],
        attributes: &'a [u8],
    },
    End,
    /// Character data without entity references.
    Text(&'a [u8]),
    /// The name of an entity or character reference, between `&` and `;`.
    Reference(&'a [u8]),
    CData(&'a [u8]),
    DocType,
    Other,
    Eof,
}

/// Yields tokens from streamed or in-memory input.
trait Tokens {
    fn next_token(&mut self) -> Result<Token<'_>>;
    /// Skips the element just started when it ended on or before `date`, and returns
    /// whether it did. Only input that can look ahead does this.
    fn skip_ended(&mut self, _date: &str) -> bool {
        false
    }
    /// Where the last token ended or an error occurred, in bytes.
    fn position(&self) -> u64;
}

/// Streamed input, read through quick-xml. Tokens are copied out of its events.
struct Streamed<R> {
    reader: Reader<R>,
    buffer: Vec<u8>,
    token: Vec<u8>,
}

impl<R: BufRead> Tokens for Streamed<R> {
    fn next_token(&mut self) -> Result<Token<'_>> {
        enum Shape {
            Start(usize),
            Text,
            CData,
            Reference,
        }
        self.buffer.clear();
        self.token.clear();
        let shape = match self.reader.read_event_into(&mut self.buffer)? {
            Event::Start(e) => {
                self.token.extend_from_slice(e.as_bytes());
                Shape::Start(e.name().as_ref().len())
            }
            Event::Text(e) => {
                self.token.extend_from_slice(e.as_bytes());
                Shape::Text
            }
            Event::CData(e) => {
                self.token.extend_from_slice(e.as_bytes());
                Shape::CData
            }
            Event::GeneralRef(e) => {
                self.token.extend_from_slice(e.as_bytes());
                Shape::Reference
            }
            Event::End(_) => return Ok(Token::End),
            Event::DocType(_) => return Ok(Token::DocType),
            Event::Eof => return Ok(Token::Eof),
            _ => return Ok(Token::Other),
        };
        let content = &self.token[..];
        Ok(match shape {
            Shape::Start(name_length) => Token::Start {
                name: &content[..name_length],
                attributes: &content[name_length..],
            },
            Shape::Text => Token::Text(content),
            Shape::CData => Token::CData(content),
            Shape::Reference => Token::Reference(content),
        })
    }

    fn position(&self) -> u64 {
        self.reader.error_position()
    }
}

/// In-memory input, scanned directly. BAG extracts are generated, well-formed XML, so this
/// only finds markup with `memchr` and checks that end tags match their start tags.
struct Scanned<'a> {
    input: &'a [u8],
    position: usize,
    /// Names of open elements, to match end tags against.
    open: Vec<&'a [u8]>,
    /// Set after an empty element's start, whose end comes next.
    empty: bool,
    /// A search for the end tag [`Tokens::skip_ended`] last looked for. Voorkomens in one
    /// document share their element name, so it is rarely rebuilt.
    end_tag: Option<memchr::memmem::Finder<'static>>,
}

impl<'a> Scanned<'a> {
    fn scan(&mut self) -> Result<Token<'a>> {
        if std::mem::take(&mut self.empty) {
            return Ok(Token::End);
        }
        let rest = &self.input[self.position..];
        let Some(&first) = rest.first() else {
            return Ok(Token::Eof);
        };
        if first == b'&' {
            let end = memchr::memchr(b';', rest).context("unterminated entity reference")?;
            self.position += end + 1;
            return Ok(Token::Reference(&rest[1..end]));
        }
        if first != b'<' {
            let end = memchr::memchr2(b'<', b'&', rest).unwrap_or(rest.len());
            self.position += end;
            return Ok(Token::Text(&rest[..end]));
        }
        let find = |needle: &[u8]| {
            memchr::memmem::find(rest, needle).with_context(|| {
                format!(
                    "unterminated XML markup, expected {}",
                    needle.escape_ascii()
                )
            })
        };
        match rest.get(1) {
            Some(b'/') => {
                let open = self.open.pop().context("unexpected XML end tag")?;
                // Usually the end tag is exactly the open element's name.
                let after = 2 + open.len();
                if rest.get(2..after) == Some(open) && rest.get(after) == Some(&b'>') {
                    self.position += after + 1;
                    return Ok(Token::End);
                }
                let end = memchr::memchr(b'>', rest).context("unterminated end tag")?;
                let name = rest[2..end].trim_ascii_end();
                ensure!(
                    open == name,
                    "end tag </{}> does not match <{}>",
                    name.escape_ascii(),
                    open.escape_ascii()
                );
                self.position += end + 1;
                return Ok(Token::End);
            }
            Some(b'!') if rest.starts_with(b"<!--") => {
                self.position += find(b"-->")? + 3;
                return Ok(Token::Other);
            }
            Some(b'!') if rest.starts_with(b"<![CDATA[") => {
                let end = find(b"]]>")?;
                self.position += end + 3;
                return Ok(Token::CData(&rest[9..end]));
            }
            Some(b'!') => return Ok(Token::DocType),
            Some(b'?') => {
                self.position += find(b"?>")? + 2;
                return Ok(Token::Other);
            }
            _ => (),
        }
        // A start tag: its name, then attributes up to the first '>' outside quotes.
        let name_end = rest[1..]
            .iter()
            .position(|&b| b == b'>' || b == b'/' || b.is_ascii_whitespace())
            .context("unterminated start tag")?
            + 1;
        let name = &rest[1..name_end];
        ensure!(!name.is_empty(), "XML tag without a name");
        let end = if rest[name_end] == b'>' {
            name_end
        } else {
            let mut from = name_end;
            loop {
                let at = memchr::memchr3(b'>', b'"', b'\'', &rest[from..])
                    .context("unterminated start tag")?
                    + from;
                if rest[at] == b'>' {
                    break at;
                }
                from = memchr::memchr(rest[at], &rest[at + 1..])
                    .context("unterminated attribute value")?
                    + at
                    + 2;
            }
        };
        let empty = rest[end - 1] == b'/' && end > name_end;
        let attributes_end = if empty { end - 1 } else { end };
        if empty {
            self.empty = true;
        } else {
            self.open.push(name);
        }
        self.position += end + 1;
        Ok(Token::Start {
            name,
            attributes: &rest[name_end..attributes_end],
        })
    }
}

impl Tokens for Scanned<'_> {
    fn next_token(&mut self) -> Result<Token<'_>> {
        self.scan()
    }

    /// Looks ahead to the element's end tag for an `eindGeldigheid` on or before `date`.
    /// Elements must not contain elements of the same name, which BAG voorkomens never do.
    fn skip_ended(&mut self, date: &str) -> bool {
        static END: std::sync::LazyLock<memchr::memmem::Finder<'static>> =
            std::sync::LazyLock::new(|| memchr::memmem::Finder::new(b"eindGeldigheid>"));
        let Some(name) = self.open.last() else {
            return false;
        };
        let needle = self.end_tag.as_ref().map(|finder| finder.needle());
        if needle.is_none_or(|needle| {
            needle.len() != name.len() + 3 || &needle[2..needle.len() - 1] != *name
        }) {
            let tag = [&b"</"[..], name, b">"].concat();
            self.end_tag = Some(memchr::memmem::Finder::new(&tag).into_owned());
        }
        let end_tag = self.end_tag.as_ref().expect("set above");
        let rest = &self.input[self.position..];
        // Without an end tag, parsing reports the problem.
        let Some(length) = end_tag.find(rest) else {
            return false;
        };
        let tag_length = end_tag.needle().len();
        let body = &rest[..length];
        let ended = END.find_iter(body).any(|at| {
            let tag_start = at.checked_sub(1).map(|i| body[i]);
            let value = body[at + 15..].split(|&b| b == b'<').next().unwrap_or(b"");
            matches!(tag_start, Some(b':' | b'<'))
                && value.len() == 10
                && value <= date.as_bytes()
                && validate_date(std::str::from_utf8(value).unwrap_or("")).is_ok()
        });
        if ended {
            self.open.pop();
            self.position += length + tag_length;
        }
        ended
    }

    fn position(&self) -> u64 {
        self.position as u64
    }
}

/// The value of attribute `key` (compared without namespace prefix) in a start tag's
/// attribute text.
fn attribute<'a>(mut attributes: &'a [u8], key: &[u8]) -> Result<Option<&'a [u8]>> {
    loop {
        attributes = attributes.trim_ascii_start();
        if attributes.is_empty() {
            return Ok(None);
        }
        let equals = memchr::memchr(b'=', attributes).context("malformed XML attribute")?;
        let name = attributes[..equals].trim_ascii();
        let rest = attributes[equals + 1..].trim_ascii_start();
        let quote = *rest.first().context("malformed XML attribute")?;
        ensure!(quote == b'"' || quote == b'\'', "unquoted XML attribute");
        let close = memchr::memchr(quote, &rest[1..]).context("unterminated attribute value")? + 1;
        if local(name) == key {
            return Ok(Some(&rest[1..close]));
        }
        attributes = &rest[close + 1..];
    }
}

/// A numeric attribute, when present.
fn number_attribute(attributes: &[u8], key: &[u8]) -> Result<Option<usize>> {
    attribute(attributes, key)?
        .map(|value| Ok(std::str::from_utf8(value)?.trim().parse()?))
        .transpose()
}

/// Appends text, normalizing line breaks as XML requires.
fn push_text(text: &mut String, bytes: &[u8]) -> Result<()> {
    let value = std::str::from_utf8(bytes)?;
    if value.contains('\r') {
        text.push_str(&value.replace("\r\n", "\n").replace('\r', "\n"));
    } else {
        text.push_str(value);
    }
    Ok(())
}

/// Appends the character an entity or character reference stands for.
fn push_reference(text: &mut String, name: &[u8]) -> Result<()> {
    let character = match name {
        b"amp" => '&',
        b"lt" => '<',
        b"gt" => '>',
        b"quot" => '"',
        b"apos" => '\'',
        [b'#', b'x' | b'X', hex @ ..] => {
            char::from_u32(u32::from_str_radix(std::str::from_utf8(hex)?, 16)?)
                .context("invalid character reference")?
        }
        [b'#', decimal @ ..] => char::from_u32(std::str::from_utf8(decimal)?.parse()?)
            .context("invalid character reference")?,
        _ => bail!("unknown entity &{};", name.escape_ascii()),
    };
    text.push(character);
    Ok(())
}

#[derive(Debug, Default)]
pub struct ParseSummary {
    pub records: u64,
    /// The extract's `StandTechnischeDatum`, when the file declares one.
    pub extract_date: Option<String>,
}

/// Reads one voorkomen at a time; memory does not grow with the number of voorkomens.
pub fn parse<R: BufRead>(input: R, emit: impl FnMut(Record) -> Result<()>) -> Result<ParseSummary> {
    let mut reader = Reader::from_reader(input);
    reader.config_mut().expand_empty_elements = true;
    records(
        Streamed {
            reader,
            buffer: Vec::with_capacity(8192),
            token: Vec::with_capacity(8192),
        },
        None,
        emit,
    )
}

/// Parses a document held in memory, scanning it in place. With `date`, voorkomens whose
/// validity ended on or before it are skipped unread, since they cannot be current; they
/// still count in [`ParseSummary::records`].
pub fn parse_bytes(
    input: &[u8],
    date: Option<&str>,
    emit: impl FnMut(Record) -> Result<()>,
) -> Result<ParseSummary> {
    records(
        Scanned {
            input,
            position: 0,
            open: Vec::with_capacity(16),
            empty: false,
            end_tag: None,
        },
        date,
        emit,
    )
}

fn records(
    mut tokens: impl Tokens,
    date: Option<&str>,
    mut emit: impl FnMut(Record) -> Result<()>,
) -> Result<ParseSummary> {
    let mut stack: Vec<&str> = Vec::with_capacity(16);
    let mut text = String::with_capacity(256);
    let mut record: Option<(usize, Record)> = None;
    let mut ring_attributes = (None, None);
    let mut summary = ParseSummary::default();
    loop {
        let token = match tokens.next_token() {
            Ok(token) => token,
            Err(error) => {
                let position = tokens.position();
                return Err(error).with_context(|| format!("XML at byte {position}"));
            }
        };
        // Whether this token starts a voorkomen, which may then be skipped.
        let mut started = false;
        // Text of elements with unknown names is never used.
        let wanted = || {
            record.is_some() && stack.last() != Some(&"")
                || stack.last() == Some(&"StandTechnischeDatum")
        };
        match token {
            Token::Start { name, attributes } => {
                let name = tag(local(name));
                // BAG objects sit in bagObject elements; relations between woonplaatsen and
                // gemeenten in their product element.
                let container = match name {
                    "GemeenteWoonplaatsRelatie" => "GemeenteWoonplaatsRelatieProduct",
                    _ => "bagObject",
                };
                if record.is_none()
                    && stack.last().is_some_and(|p| *p == container)
                    && let Some(kind) = Kind::from_tag(name)
                {
                    started = true;
                    record = Some((
                        stack.len(),
                        Record {
                            kind,
                            id: String::new(),
                            validity: Validity {
                                // Relations always state their validity period.
                                has_history: kind == Kind::GemeenteWoonplaatsRelatie,
                                ..Validity::default()
                            },
                            fields: Fields::default(),
                        },
                    ));
                }
                if let Some((_, r)) = &mut record {
                    if name == "Voorkomen" {
                        r.validity.has_history = true;
                    }
                    if matches!(name, "Point" | "Polygon" | "posList") {
                        let dimension = number_attribute(attributes, b"srsDimension")?;
                        let count = number_attribute(attributes, b"count")?;
                        match name {
                            "Point" => r.fields.point_dimension = dimension,
                            "Polygon" => r.fields.polygon_dimension = dimension,
                            _ => ring_attributes = (count, dimension),
                        }
                    }
                }
                stack.push(name);
                text.clear();
            }
            Token::Text(bytes) | Token::CData(bytes) => {
                if wanted() {
                    push_text(&mut text, bytes)?;
                }
            }
            Token::Reference(name) => {
                if wanted() {
                    push_reference(&mut text, name)?;
                }
            }
            Token::End => {
                ensure!(!stack.is_empty(), "unexpected XML end tag");
                if stack.last() == Some(&"StandTechnischeDatum") && record.is_none() {
                    let date = text.trim();
                    validate_date(date).context("StandTechnischeDatum")?;
                    summary.extract_date = Some(date.into());
                }
                if let Some((start, r)) = &mut record {
                    let path = &stack[*start..];
                    // Woonplaats boundaries are large and unused, so their rings are skipped.
                    if path.last() == Some(&"posList")
                        && r.kind != Kind::Woonplaats
                        && path.contains(&"geometrie")
                    {
                        let exterior = path.contains(&"exterior");
                        if exterior || path.contains(&"interior") {
                            ensure!(
                                exterior == r.fields.rings.is_empty(),
                                "{} {} needs one exterior ring before its interior rings",
                                r.kind,
                                r.id
                            );
                            let (count, dimension) = std::mem::take(&mut ring_attributes);
                            r.fields.rings.push(RawRing {
                                text: std::mem::take(&mut text),
                                count,
                                dimension,
                            });
                        }
                    } else if path.last() != Some(&"") {
                        r.field(path, &text).with_context(|| {
                            format!("{} {} field {}", r.kind, r.id, stack.last().unwrap())
                        })?;
                    }
                    if stack.len() == *start + 1 {
                        let (_, r) = record.take().unwrap();
                        ensure!(!r.id.is_empty(), "{} missing identification", r.kind);
                        emit(r)?;
                        summary.records += 1;
                    }
                }
                stack.pop();
                text.clear();
            }
            Token::DocType => bail!("DTD declarations are not supported in BAG extracts"),
            Token::Eof => {
                ensure!(stack.is_empty(), "truncated XML");
                break;
            }
            Token::Other => (),
        }
        if started && date.is_some_and(|date| tokens.skip_ended(date)) {
            record = None;
            stack.pop();
            summary.records += 1;
        }
    }
    Ok(summary)
}
