//! The HTTP API's resources, independent of the HTTP server: [`route`] turns a request
//! into a [`Route`] or an error response, and [`respond`] answers a route from a
//! database. The OpenAPI description in `openapi.json` documents the same contract. The
//! design follows the Dutch government's API Design Rules (ADR-0007), so resources,
//! fields, parameters and messages are in Dutch, with the BAG's terms (ADR-0008).

use crate::{
    geo::wgs84_to_rd,
    query::{self, Address, AddressQuery, Database, Match},
    xml::bag_id,
};
use anyhow::Result;
use serde_json::{Value as Json, json};

/// The API version, which `info.version` in the OpenAPI description repeats. Bump it
/// with every change to the API: the major version for incompatible ones, which also
/// changes [`BASE_PATH`].
pub const VERSION: &str = "1.0.1";

/// Where the API lives; the major version of [`VERSION`].
pub const BASE_PATH: &str = "/v1";

/// The OpenAPI description, served at `/v1/openapi.json`.
pub const OPENAPI: &str = include_str!("openapi.json");

const JSON: &str = "application/json";

/// Why an adreslijst cannot answer a nearest lookup.
const NEAREST_NEEDS_TABLES: &str =
    "zoeken op afstand vereist een database met tabellen; deze database is een adreslijst";
const PROBLEM_JSON: &str = "application/problem+json";

/// Every error status the API answers with and when; problem details carry a specific
/// `detail`. The documentation's HTTP error table is generated from this list.
pub const ERRORS: &[(u16, &str)] = &[
    (
        400,
        "A parameter is missing, unknown, repeated or invalid; `invalidParams` lists each problem",
    ),
    (
        404,
        "The path is not a resource, or no nummeraanduiding has the identification",
    ),
    (405, "Anything but `GET` or `HEAD`; `Allow` lists both"),
    (406, "`Accept` rules out `application/json`"),
    (500, "The query failed; the server logs why"),
    (
        501,
        "A nearest lookup against an address list, which has no spatial index",
    ),
    (
        503,
        "The database workers are not available, for example while the server stops",
    ),
];

/// A coordinate reference system the API reads and writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Crs {
    /// WGS84 longitude and latitude, the default.
    Crs84,
    /// RD New (EPSG:28992), in metres.
    RdNew,
}

impl Crs {
    pub const ALL: [Crs; 2] = [Crs::Crs84, Crs::RdNew];

    pub fn uri(self) -> &'static str {
        match self {
            Crs::Crs84 => "http://www.opengis.net/def/crs/OGC/1.3/CRS84",
            Crs::RdNew => "http://www.opengis.net/def/crs/EPSG/0/28992",
        }
    }

    fn from_uri(uri: &str) -> Option<Crs> {
        Crs::ALL.into_iter().find(|crs| crs.uri() == uri)
    }
}

/// A request the API accepted, ready to be answered.
#[derive(Debug, Clone)]
pub enum Route {
    Root,
    OpenApi,
    Crss,
    Addresses {
        query: AddressQuery,
        crs: Crs,
    },
    Address {
        id: i64,
        crs: Crs,
    },
    /// A punt in RD New, or `None` when a WGS84 punt lies outside RD New's area.
    Nearest {
        point: Option<(f64, f64)>,
        crs: Crs,
    },
}

/// A complete response: status, headers and body.
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: String,
}

impl Response {
    fn new(status: u16, content_type: &str, body: String) -> Response {
        let mut headers = vec![("Content-Type", content_type.to_owned())];
        headers.extend(
            [
                ("API-Version", VERSION),
                ("Cache-Control", "no-store"),
                (
                    "Content-Security-Policy",
                    "default-src 'none'; frame-ancestors 'none'",
                ),
                ("Strict-Transport-Security", "max-age=31536000"),
                ("X-Content-Type-Options", "nosniff"),
                ("X-Frame-Options", "DENY"),
                ("Referrer-Policy", "no-referrer"),
                ("Access-Control-Allow-Origin", "*"),
                ("Access-Control-Expose-Headers", "API-Version, Content-Crs"),
            ]
            .map(|(name, value)| (name, value.to_owned())),
        );
        Response {
            status,
            headers,
            body,
        }
    }

    fn json(value: &Json) -> Response {
        Response::new(200, JSON, value.to_string())
    }

    fn with_header(mut self, name: &'static str, value: String) -> Response {
        self.headers.push((name, value));
        self
    }

    /// The value of a header, if the response has it.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// One reason a parameter was rejected.
struct InvalidParam {
    name: String,
    code: &'static str,
    reason: String,
}

/// The problem title for a status: its reason phrase, in Dutch.
pub fn title(status: u16) -> &'static str {
    match status {
        400 => "Ongeldige aanvraag",
        404 => "Niet gevonden",
        405 => "Methode niet toegestaan",
        406 => "Niet aanvaardbaar",
        500 => "Interne serverfout",
        501 => "Niet geïmplementeerd",
        503 => "Dienst niet beschikbaar",
        _ => "Fout",
    }
}

/// An RFC 9457 problem details response.
pub fn problem(status: u16, detail: &str) -> Response {
    problem_with(status, detail, &[])
}

fn problem_with(status: u16, detail: &str, invalid: &[InvalidParam]) -> Response {
    let mut body = json!({
        "status": status,
        "title": title(status),
        "detail": detail,
    });
    if !invalid.is_empty() {
        body["invalidParams"] = invalid
            .iter()
            .map(|p| json!({ "name": p.name, "code": p.code, "reason": p.reason }))
            .collect();
    }
    Response::new(status, PROBLEM_JSON, body.to_string())
}

/// Each resource as the OpenAPI description names it: its path below [`BASE_PATH`], and
/// the query parameters it accepts with whether each is required. [`route`] accepts
/// exactly these, and `tests/openapi.rs` checks them against the description.
pub const RESOURCES: &[(&str, &[(&str, bool)])] = &[
    ("/", &[]),
    ("/openapi.json", &[]),
    ("/crss", &[]),
    (
        "/adressen",
        &[
            ("postcode", true),
            ("huisnummer", false),
            ("huisletter", false),
            ("huisnummertoevoeging", false),
            ("exacteMatch", false),
            ("crs", false),
        ],
    ),
    (
        "/adressen/_dichtstbij",
        &[("punt", true), ("puntCrs", false), ("crs", false)],
    ),
    (
        "/adressen/{nummeraanduidingIdentificatie}",
        &[("crs", false)],
    ),
];

/// Routes a request from its method, request target (path and query) and `Accept`
/// header. Returns the response straight away when the request cannot be answered.
pub fn route(method: &str, target: &str, accept: Option<&str>) -> Result<Route, Response> {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let Some(rest) = path.strip_prefix(BASE_PATH) else {
        return Err(problem(
            404,
            &format!("{path} is geen resource; de API staat op {BASE_PATH}"),
        ));
    };
    let resource = match rest {
        "" | "/" => Resource::Root,
        "/openapi.json" => Resource::OpenApi,
        "/crss" => Resource::Crss,
        "/adressen" => Resource::Addresses,
        "/adressen/_dichtstbij" => Resource::Nearest,
        _ => match rest.strip_prefix("/adressen/") {
            Some(id) if !id.is_empty() && !id.contains('/') => Resource::Address(id),
            _ => return Err(problem(404, &format!("{path} is geen resource"))),
        },
    };
    if method != "GET" && method != "HEAD" {
        return Err(
            problem(405, &format!("{path} ondersteunt alleen GET en HEAD"))
                .with_header("Allow", "GET, HEAD".into()),
        );
    }
    if let Some(accept) = accept
        && !accepts_json(accept)
    {
        return Err(problem(
            406,
            &format!("{path} is alleen beschikbaar als {JSON}"),
        ));
    }
    let mut params = Params::parse(query, resource.path());
    let route = match resource {
        Resource::Root => Route::Root,
        Resource::OpenApi => Route::OpenApi,
        Resource::Crss => Route::Crss,
        Resource::Addresses => {
            let postcode = params.take("postcode", parse_postcode);
            let house_number = params.take("huisnummer", parse_house_number);
            let letter = params.take("huisletter", parse_house_letter);
            let addition = params.take("huisnummertoevoeging", parse_addition);
            let exact = params.take("exacteMatch", parse_bool);
            let crs = params.take("crs", parse_crs).unwrap_or(Crs::Crs84);
            let exact = exact.unwrap_or(false);
            if exact && house_number.is_none() && params.ok("huisnummer") {
                params.invalid(
                    "exacteMatch",
                    "conflict",
                    "exacteMatch=true vereist huisnummer".into(),
                );
            }
            let matching = |value: Option<String>| match value {
                Some(value) => Match::Equals(value),
                None if exact => Match::Absent,
                None => Match::Any,
            };
            Route::Addresses {
                query: AddressQuery {
                    postcode: postcode.unwrap_or_default(),
                    house_number,
                    house_letter: matching(letter),
                    house_number_addition: matching(addition),
                },
                crs,
            }
        }
        Resource::Address(id) => {
            let crs = params.take("crs", parse_crs).unwrap_or(Crs::Crs84);
            let id = match parse_address_id(id) {
                Ok(id) => id,
                Err(reason) => {
                    params.invalid("nummeraanduidingIdentificatie", "invalid", reason);
                    0
                }
            };
            Route::Address { id, crs }
        }
        Resource::Nearest => {
            let point = params.take("punt", parse_pair);
            let point_crs = params.take("puntCrs", parse_crs).unwrap_or(Crs::Crs84);
            let crs = params.take("crs", parse_crs).unwrap_or(Crs::Crs84);
            let point = match (point, point_crs) {
                (Some((lon, lat)), Crs::Crs84) => {
                    if (-180.0..=180.0).contains(&lon) && (-90.0..=90.0).contains(&lat) {
                        wgs84_to_rd(lat, lon)
                    } else {
                        params.invalid(
                            "punt",
                            "invalid",
                            "moet een lengtegraad van -180 tot 180 en een breedtegraad van -90 tot 90 zijn"
                                .into(),
                        );
                        None
                    }
                }
                (point, _) => point,
            };
            Route::Nearest { point, crs }
        }
    };
    params.finish()?;
    Ok(route)
}

enum Resource<'a> {
    Root,
    OpenApi,
    Crss,
    Addresses,
    Address(&'a str),
    Nearest,
}

impl Resource<'_> {
    /// The path as [`RESOURCES`] and the OpenAPI description write it.
    fn path(&self) -> &'static str {
        match self {
            Resource::Root => "/",
            Resource::OpenApi => "/openapi.json",
            Resource::Crss => "/crss",
            Resource::Addresses => "/adressen",
            Resource::Address(_) => "/adressen/{nummeraanduidingIdentificatie}",
            Resource::Nearest => "/adressen/_dichtstbij",
        }
    }
}

/// Answers a route that needs no database, such as the OpenAPI description.
pub fn respond_static(route: &Route) -> Option<Response> {
    match route {
        Route::OpenApi => Some(Response::new(200, JSON, OPENAPI.to_owned())),
        _ => None,
    }
}

/// Answers a route. Fails only when the database query fails.
pub fn respond(db: &Database, route: &Route) -> Result<Response> {
    if let Some(response) = respond_static(route) {
        return Ok(response);
    }
    let geometry = |response: Response, crs: Crs| {
        response.with_header("Content-Crs", format!("<{}>", crs.uri()))
    };
    Ok(match route {
        Route::Root => Response::json(&root(db)?),
        Route::OpenApi => unreachable!("answered without the database"),
        Route::Crss => Response::json(&json!({
            "crs": Crs::ALL.map(Crs::uri),
            "storageCrs": if db.is_address_list() { Crs::Crs84 } else { Crs::RdNew }.uri(),
        })),
        Route::Addresses { query, crs } => {
            let addresses = query::find_addresses(db, query)?;
            geometry(collection(&addresses, *crs, None), *crs)
        }
        Route::Address { id, crs } => match query::address(db, *id)? {
            Some(address) => geometry(Response::json(&address_json(&address, *crs, None)), *crs),
            None => problem(
                404,
                &format!(
                    "geen nummeraanduiding met identificatie {}",
                    bag_id(*id, 16)
                ),
            ),
        },
        Route::Nearest { point, crs } => {
            let addresses = match point {
                Some((x, y)) => match query::nearest_addresses(db, *x, *y) {
                    Ok(addresses) => addresses,
                    Err(error) if error.is::<query::Unsupported>() => {
                        return Ok(problem(501, NEAREST_NEEDS_TABLES));
                    }
                    Err(error) => return Err(error),
                },
                None if db.is_address_list() => {
                    return Ok(problem(501, NEAREST_NEEDS_TABLES));
                }
                None => Vec::new(),
            };
            geometry(collection(&addresses, *crs, *point), *crs)
        }
    })
}

/// Routes and answers a `GET` request, as `bagkit query` does.
pub fn get(db: &Database, target: &str) -> Result<Response> {
    match route("GET", target, None) {
        Ok(route) => respond(db, &route),
        Err(response) => Ok(response),
    }
}

fn root(db: &Database) -> Result<Json> {
    let layout = if db.is_address_list() {
        "adreslijst"
    } else {
        "tabellen"
    };
    Ok(json!({
        "titel": "bagkit",
        "beschrijving": "Adressen uit een momentopname van de Basisregistratie Adressen en Gebouwen (BAG).",
        "versie": VERSION,
        "dataset": {
            "indeling": layout,
            "peildatum": db.metadata("reference_date")?,
            "extractdatum": db.metadata("extract_date")?,
        },
        "links": [
            { "rel": "self", "href": BASE_PATH, "type": JSON },
            {
                "rel": "service-desc",
                "href": format!("{BASE_PATH}/openapi.json"),
                "type": "application/vnd.oai.openapi+json;version=3.0",
            },
        ],
    }))
}

fn collection(addresses: &[Address], crs: Crs, origin: Option<(f64, f64)>) -> Response {
    let items: Vec<Json> = addresses
        .iter()
        .map(|address| address_json(address, crs, origin))
        .collect();
    Response::json(&json!({ "adressen": items }))
}

/// Rounds to a number of decimals: millimetres in RD New and about a millimetre in WGS84.
fn round(value: f64, decimals: i32) -> f64 {
    let scale = 10f64.powi(decimals);
    (value * scale).round() / scale
}

fn address_json(address: &Address, crs: Crs, origin: Option<(f64, f64)>) -> Json {
    let mut result = json!({
        "nummeraanduidingIdentificatie": bag_id(address.id, 16),
        "postcode": address.postcode,
        "huisnummer": address.house_number,
        "huisletter": address.house_letter,
        "huisnummertoevoeging": address.house_number_addition,
        "openbareRuimteNaam": address.street,
        "openbareRuimteIdentificatie": bag_id(address.public_space_id, 16),
        "woonplaatsNaam": address.locality,
    });
    if let Some(id) = address.locality_id {
        result["woonplaatsIdentificatie"] = json!(bag_id(id, 4));
    }
    result["gemeenteNaam"] = json!(address.municipality);
    result["provincieNaam"] = json!(address.province);
    if let Some(status) = &address.status {
        result["status"] = json!(status);
    }
    let Some(object) = &address.object else {
        result["adresseerbaarObject"] = Json::Null;
        return result;
    };
    if let Some(secondary) = object.is_secondary_address {
        result["indicatieNevenadres"] = json!(secondary);
    }
    let mut item = json!({});
    if let Some(id) = object.id {
        item["identificatie"] = json!(bag_id(id, 16));
    }
    item["type"] = json!(object.kind);
    if let Some(status) = &object.status {
        item["status"] = json!(status);
    }
    if let Some(purposes) = &object.usage_purposes {
        item["gebruiksdoelen"] = json!(purposes);
    }
    if object.id.is_some() {
        item["oppervlakte"] = json!(object.usable_area_m2);
    }
    let coordinates = match crs {
        Crs::Crs84 => object
            .lat_lon
            .map(|(lat, lon)| [round(lon, 8), round(lat, 8)]),
        Crs::RdNew => object.rd_point().map(|(x, y)| [round(x, 3), round(y, 3)]),
    };
    item["geometrie"] = match coordinates {
        Some(coordinates) => json!({ "type": "Point", "coordinates": coordinates }),
        None => Json::Null,
    };
    if let Some(buildings) = &object.buildings {
        item["panden"] = buildings
            .iter()
            .map(|b| {
                json!({
                    "identificatie": bag_id(b.id, 16),
                    "oorspronkelijkBouwjaar": b.construction_year,
                    "status": b.status,
                })
            })
            .collect();
    }
    result["adresseerbaarObject"] = item;
    if let (Some((x, y)), Some((px, py))) = (origin, object.rd_point()) {
        result["afstand"] = json!(round((px - x).hypot(py - y), 3));
    }
    result
}

/// Whether an `Accept` header allows JSON. Media ranges with `q=0` are refused.
fn accepts_json(accept: &str) -> bool {
    accept.split(',').any(|range| {
        let mut parts = range.split(';');
        let media = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        let refused = parts.any(|p| {
            p.trim()
                .strip_prefix("q=")
                .and_then(|q| q.trim().parse::<f64>().ok())
                == Some(0.0)
        });
        !refused && matches!(media.as_str(), JSON | "application/*" | "*/*")
    })
}

/// Query parameters, consumed one name at a time so that unknown and repeated ones and
/// every invalid value are reported together.
struct Params {
    pairs: Vec<(Option<String>, Option<String>)>,
    invalid: Vec<InvalidParam>,
    /// The resource's parameters from [`RESOURCES`].
    declared: &'static [(&'static str, bool)],
}

impl Params {
    fn parse(query: &str, path: &str) -> Params {
        let pairs = query
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|pair| {
                let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
                (percent_decode(name), percent_decode(value))
            })
            .collect();
        let declared = RESOURCES
            .iter()
            .find(|(p, _)| *p == path)
            .map_or(&[][..], |(_, declared)| *declared);
        Params {
            pairs,
            invalid: Vec::new(),
            declared,
        }
    }

    fn invalid(&mut self, name: &str, code: &'static str, reason: String) {
        self.invalid.push(InvalidParam {
            name: name.to_owned(),
            code,
            reason,
        });
    }

    /// Whether no problem was recorded for a parameter.
    fn ok(&self, name: &str) -> bool {
        self.invalid.iter().all(|p| p.name != name)
    }

    /// Removes a declared parameter and parses its value, recording why when it cannot.
    fn take<T>(&mut self, name: &str, parse: impl Fn(&str) -> Result<T, String>) -> Option<T> {
        let Some(&(_, required)) = self.declared.iter().find(|(n, _)| *n == name) else {
            panic!("{name} is not declared in api::RESOURCES");
        };
        let mut values = Vec::new();
        self.pairs.retain(|(n, v)| {
            if n.as_deref() == Some(name) {
                values.push(v.clone());
                false
            } else {
                true
            }
        });
        match values.as_slice() {
            [] if required => self.invalid(name, "required", "is verplicht".into()),
            [] => (),
            [Some(value)] => match parse(value) {
                Ok(value) => return Some(value),
                Err(reason) => self.invalid(name, "invalid", reason),
            },
            [None] => self.invalid(name, "invalid", "is geen geldige UTF-8".into()),
            _ => self.invalid(name, "repeated", "is meer dan eens opgegeven".into()),
        }
        None
    }

    fn finish(mut self) -> Result<(), Response> {
        for (name, _) in std::mem::take(&mut self.pairs) {
            let name = name.unwrap_or_else(|| "?".into());
            if self.invalid.iter().all(|p| p.name != name) {
                self.invalid(
                    &name,
                    "unknown",
                    "is geen parameter van deze resource".into(),
                );
            }
        }
        if self.invalid.is_empty() {
            return Ok(());
        }
        let names: Vec<_> = self.invalid.iter().map(|p| p.name.as_str()).collect();
        Err(problem_with(
            400,
            &format!("ongeldige parameters: {}", names.join(", ")),
            &self.invalid,
        ))
    }
}

/// Decodes a query component: `+` is a space and `%XX` a byte. `None` for malformed
/// escapes or bytes that are not UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut input = text.bytes();
    while let Some(byte) = input.next() {
        bytes.push(match byte {
            b'+' => b' ',
            b'%' => {
                let high = (input.next()? as char).to_digit(16)?;
                let low = (input.next()? as char).to_digit(16)?;
                (high * 16 + low) as u8
            }
            byte => byte,
        });
    }
    String::from_utf8(bytes).ok()
}

fn parse_postcode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    if bytes.len() == 6
        && (b'1'..=b'9').contains(&bytes[0])
        && bytes[1..4].iter().all(u8::is_ascii_digit)
        && bytes[4..].iter().all(u8::is_ascii_alphabetic)
    {
        Ok(value.to_ascii_uppercase())
    } else {
        Err("moet bestaan uit vier cijfers, waarvan het eerste geen 0 is, en twee letters, zoals 7311KZ".into())
    }
}

fn parse_house_number(value: &str) -> Result<i64, String> {
    match value.parse::<i64>() {
        Ok(number)
            if (1..=99999).contains(&number) && value.bytes().all(|b| b.is_ascii_digit()) =>
        {
            Ok(number)
        }
        _ => Err("moet een geheel getal van 1 tot en met 99999 zijn".into()),
    }
}

fn parse_house_letter(value: &str) -> Result<String, String> {
    if value.len() == 1 && value.bytes().all(|b| b.is_ascii_alphabetic()) {
        Ok(value.to_owned())
    } else {
        Err("moet één letter zijn".into())
    }
}

fn parse_addition(value: &str) -> Result<String, String> {
    if (1..=4).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Ok(value.to_owned())
    } else {
        Err("moet één tot vier letters of cijfers zijn".into())
    }
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err("moet true of false zijn".into()),
    }
}

fn parse_crs(value: &str) -> Result<Crs, String> {
    Crs::from_uri(value).ok_or_else(|| format!("moet {} zijn", Crs::ALL.map(Crs::uri).join(" of ")))
}

fn parse_pair(value: &str) -> Result<(f64, f64), String> {
    let reason =
        || "moet twee decimale getallen zijn, gescheiden door een komma, zoals 5.9624,52.2117";
    let (first, second) = value.split_once(',').ok_or_else(reason)?;
    let number = |text: &str| {
        text.parse::<f64>()
            .ok()
            .filter(|n| n.is_finite() && !text.contains(['e', 'E', 'i', 'n', 'N']))
    };
    number(first)
        .zip(number(second))
        .ok_or_else(|| reason().to_owned())
}

fn parse_address_id(value: &str) -> Result<i64, String> {
    if value.len() == 16 && value.bytes().all(|b| b.is_ascii_digit()) {
        Ok(value.parse().unwrap())
    } else {
        Err("moet een identificatie van 16 cijfers zijn, zoals 0003200000137381".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invalid_names(response: Response) -> Vec<(String, String)> {
        assert_eq!(response.status, 400);
        let body: Json = serde_json::from_str(&response.body).unwrap();
        body["invalidParams"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    p["name"].as_str().unwrap().to_owned(),
                    p["code"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn routes_resources_under_the_base_path() {
        assert!(matches!(route("GET", "/v1", None), Ok(Route::Root)));
        assert!(matches!(route("GET", "/v1/", None), Ok(Route::Root)));
        assert!(matches!(
            route("HEAD", "/v1/openapi.json", None),
            Ok(Route::OpenApi)
        ));
        for path in [
            "/",
            "/v1/adressen/",
            "/v2/adressen",
            "/v1/crss/",
            "/7311KZ/110",
        ] {
            assert_eq!(route("GET", path, None).unwrap_err().status, 404, "{path}");
        }
        let response = route("POST", "/v1/adressen", None).unwrap_err();
        assert_eq!(response.status, 405);
        assert_eq!(response.header("Allow"), Some("GET, HEAD"));
    }

    #[test]
    fn address_queries_normalize_postcodes_and_honour_exact_matches() {
        let Ok(Route::Addresses { query, crs }) = route(
            "GET",
            "/v1/adressen?postcode=7311kz&huisnummer=110&huisnummertoevoeging=2&exacteMatch=true",
            None,
        ) else {
            panic!("not an address query");
        };
        assert_eq!(query.postcode, "7311KZ");
        assert_eq!(query.house_number, Some(110));
        assert_eq!(query.house_letter, Match::Absent);
        assert_eq!(query.house_number_addition, Match::Equals("2".into()));
        assert_eq!(crs, Crs::Crs84);
        let Ok(Route::Addresses { query, .. }) =
            route("GET", "/v1/adressen?postcode=7311KZ&huisnummer=110", None)
        else {
            panic!("not an address query");
        };
        assert_eq!(query.house_letter, Match::Any);
    }

    #[test]
    fn invalid_parameters_are_reported_together() {
        let response = route(
            "GET",
            "/v1/adressen?huisnummer=0&huisletter=AB&crs=EPSG:28992&houseNumber=1&exacteMatch=yes",
            None,
        )
        .unwrap_err();
        assert_eq!(response.header("Content-Type"), Some(PROBLEM_JSON));
        assert_eq!(
            invalid_names(response),
            [
                ("postcode", "required"),
                ("huisnummer", "invalid"),
                ("huisletter", "invalid"),
                ("exacteMatch", "invalid"),
                ("crs", "invalid"),
                ("houseNumber", "unknown"),
            ]
            .map(|(n, c)| (n.to_owned(), c.to_owned()))
        );
        let response = route(
            "GET",
            "/v1/adressen?postcode=7311KZ&postcode=7311KZ&exacteMatch=true",
            None,
        )
        .unwrap_err();
        assert_eq!(
            invalid_names(response),
            [("postcode", "repeated"), ("exacteMatch", "conflict")]
                .map(|(n, c)| (n.to_owned(), c.to_owned()))
        );
        let response = route("GET", "/v1/adressen/12345?crs=x", None).unwrap_err();
        assert_eq!(
            invalid_names(response),
            [
                ("crs", "invalid"),
                ("nummeraanduidingIdentificatie", "invalid")
            ]
            .map(|(n, c)| (n.to_owned(), c.to_owned()))
        );
    }

    #[test]
    fn nearest_points_are_read_in_their_crs() {
        let Ok(Route::Nearest { point, crs }) = route(
            "GET",
            "/v1/adressen/_dichtstbij?punt=194000.5%2C470000&puntCrs=http%3A%2F%2Fwww.opengis.net%2Fdef%2Fcrs%2FEPSG%2F0%2F28992&crs=http://www.opengis.net/def/crs/EPSG/0/28992",
            None,
        ) else {
            panic!("not a nearest lookup");
        };
        assert_eq!(point, Some((194000.5, 470000.0)));
        assert_eq!(crs, Crs::RdNew);
        let Ok(Route::Nearest { point, .. }) =
            route("GET", "/v1/adressen/_dichtstbij?punt=0,0", None)
        else {
            panic!("not a nearest lookup");
        };
        assert_eq!(
            point, None,
            "outside RD New finds nothing rather than failing"
        );
        for point in ["52.2,200", "5.9", "5.9,NaN", "5.9,1e3", "a,b"] {
            let target = format!("/v1/adressen/_dichtstbij?punt={point}");
            assert_eq!(
                invalid_names(route("GET", &target, None).unwrap_err()),
                [("punt".to_owned(), "invalid".to_owned())],
                "{point}"
            );
        }
    }

    #[test]
    fn accept_headers_must_allow_json() {
        for accept in [
            "application/json",
            "*/*",
            "text/html, application/*;q=0.5",
            "Application/JSON; charset=utf-8",
        ] {
            assert!(route("GET", "/v1", Some(accept)).is_ok(), "{accept}");
        }
        for accept in ["text/html", "application/json;q=0", "application/xml"] {
            let response = route("GET", "/v1", Some(accept)).unwrap_err();
            assert_eq!(response.status, 406, "{accept}");
        }
    }

    #[test]
    fn every_response_carries_the_version_and_security_headers() {
        for response in [problem(404, "x"), Response::json(&json!({}))] {
            assert_eq!(response.header("API-Version"), Some(VERSION));
            assert_eq!(response.header("Access-Control-Allow-Origin"), Some("*"));
            assert_eq!(response.header("X-Content-Type-Options"), Some("nosniff"));
            assert_eq!(response.header("Cache-Control"), Some("no-store"));
        }
    }

    #[test]
    fn openapi_description_matches_the_api_version_and_base_path() {
        let description: Json = serde_json::from_str(OPENAPI).unwrap();
        assert_eq!(description["info"]["version"], VERSION);
        assert_eq!(description["servers"][0]["url"], BASE_PATH);
        let major = VERSION.split('.').next().unwrap();
        assert_eq!(BASE_PATH, format!("/v{major}"));
        let documented: Vec<_> = ERRORS
            .iter()
            .map(|(status, _)| status.to_string())
            .collect();
        let used: std::collections::BTreeSet<String> = description["paths"]
            .as_object()
            .unwrap()
            .values()
            .flat_map(|item| {
                item["get"]["responses"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .cloned()
            })
            .filter(|status| !status.starts_with('2'))
            .collect();
        for status in used {
            assert!(documented.contains(&status), "{status} is not in ERRORS");
        }
    }
}
