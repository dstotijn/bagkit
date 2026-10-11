mod common;

use bagkit::{
    api,
    dataset::{ADDRESS_LIST, Format},
    query,
};
use common::{ImportOptions, import};
use rusqlite::{Connection, types::Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn fixture() -> String {
    fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bag.xml"))
        .unwrap()
}

fn options(output: PathBuf, address_list: Option<Format>) -> ImportOptions {
    ImportOptions {
        output,
        date: "2026-10-07".into(),
        force: false,
        cache_mib: 8,
        geometry: true,
        threads: 0,
        address_list,
    }
}

/// Relates both fixture localities to municipalities in bagkit's table.
const RELATIONS: &str = "<gwr-product:GemeenteWoonplaatsRelatieProduct xmlns:gwr-product=\"urn:p\" xmlns:bagtypes=\"urn:t\">\
    <gwr-product:GemeenteWoonplaatsRelatie><gwr-product:tijdvakgeldigheid><bagtypes:begindatumTijdvakGeldigheid>2000-01-01</bagtypes:begindatumTijdvakGeldigheid></gwr-product:tijdvakgeldigheid>\
    <gwr-product:gerelateerdeWoonplaats><gwr-product:identificatie>1234</gwr-product:identificatie></gwr-product:gerelateerdeWoonplaats>\
    <gwr-product:gerelateerdeGemeente><gwr-product:identificatie>0820</gwr-product:identificatie></gwr-product:gerelateerdeGemeente><gwr-product:status>definitief</gwr-product:status></gwr-product:GemeenteWoonplaatsRelatie>\
    <gwr-product:GemeenteWoonplaatsRelatie><gwr-product:tijdvakgeldigheid><bagtypes:begindatumTijdvakGeldigheid>2000-01-01</bagtypes:begindatumTijdvakGeldigheid></gwr-product:tijdvakgeldigheid>\
    <gwr-product:gerelateerdeWoonplaats><gwr-product:identificatie>5678</gwr-product:identificatie></gwr-product:gerelateerdeWoonplaats>\
    <gwr-product:gerelateerdeGemeente><gwr-product:identificatie>0014</gwr-product:identificatie></gwr-product:gerelateerdeGemeente><gwr-product:status>definitief</gwr-product:status></gwr-product:GemeenteWoonplaatsRelatie>\
    </gwr-product:GemeenteWoonplaatsRelatieProduct>";

/// An address without an object, on a public space whose name needs CSV quoting.
const UNLINKED: &str = "<sl:stand><sl-bag-extract:bagObject><Objecten:OpenbareRuimte><Objecten:identificatie>0000300000000002</Objecten:identificatie><Objecten:naam>Hof, 't \"Zand\"</Objecten:naam><Objecten:type>Weg</Objecten:type><Objecten:status>Naamgeving uitgegeven</Objecten:status><Objecten:ligtIn><Objecten-ref:WoonplaatsRef>5678</Objecten-ref:WoonplaatsRef></Objecten:ligtIn><Objecten:voorkomen><Historie:Voorkomen><Historie:beginGeldigheid>2000-01-01</Historie:beginGeldigheid></Historie:Voorkomen></Objecten:voorkomen></Objecten:OpenbareRuimte></sl-bag-extract:bagObject></sl:stand>\
<sl:stand><sl-bag-extract:bagObject><Objecten:Nummeraanduiding><Objecten:identificatie>0000200000000009</Objecten:identificatie><Objecten:huisnummer>1</Objecten:huisnummer><Objecten:status>Naamgeving uitgegeven</Objecten:status><Objecten:ligtAan><Objecten-ref:OpenbareRuimteRef>0000300000000002</Objecten-ref:OpenbareRuimteRef></Objecten:ligtAan><Objecten:voorkomen><Historie:Voorkomen><Historie:beginGeldigheid>2000-01-01</Historie:beginGeldigheid></Historie:Voorkomen></Objecten:voorkomen></Objecten:Nummeraanduiding></sl-bag-extract:bagObject></sl:stand>";

fn with_unlinked(xml: String) -> String {
    xml.replace(
        "</sl:standBestand>",
        &format!("{UNLINKED}</sl:standBestand>"),
    )
}

/// The fixture split the way the national extract names its files: everything except
/// addresses in one file, addresses in a file whose name contains NUM.
fn split(dir: &Path, xml: &str) -> Vec<PathBuf> {
    let (head, rest) = xml.split_once("<sl:stand>").unwrap();
    let (body, tail) = rest.rsplit_once("</sl:stand>").unwrap();
    let records: Vec<String> = format!("<sl:stand>{body}</sl:stand>")
        .split_inclusive("</sl:stand>")
        .map(str::to_owned)
        .collect();
    let (addresses, others): (Vec<_>, Vec<_>) = records
        .into_iter()
        .partition(|r| r.contains("<Objecten:Nummeraanduiding>"));
    let mut paths = vec![dir.join("GEM-WPL-RELATIE.xml")];
    fs::write(&paths[0], RELATIONS).unwrap();
    for (name, records) in [("9999OBJ.xml", others), ("9999NUM.xml", addresses)] {
        let path = dir.join(name);
        fs::write(&path, format!("{head}{}{tail}", records.concat())).unwrap();
        paths.push(path);
    }
    paths
}

/// Every address list row as text, the way the CSV writes it.
fn sqlite_rows(path: &Path) -> Vec<Vec<String>> {
    let db = Connection::open(path).unwrap();
    let mut statement = db
        .prepare("SELECT * FROM adreslijst ORDER BY nummeraanduiding_identificatie")
        .unwrap();
    statement
        .query_map([], |row| {
            (0..ADDRESS_LIST.columns.len())
                .map(|i| {
                    Ok(
                        match (ADDRESS_LIST.columns[i].name, row.get::<_, Value>(i)?) {
                            (
                                "nummeraanduiding_identificatie" | "openbare_ruimte_identificatie",
                                Value::Integer(id),
                            ) => format!("{id:016}"),
                            (_, Value::Integer(n)) => n.to_string(),
                            (_, Value::Real(r)) => format!("{r:.7}"),
                            (_, Value::Text(t)) => t,
                            _ => String::new(),
                        },
                    )
                })
                .collect()
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Parses CSV with quoted fields.
fn csv_rows(path: &Path) -> (Vec<String>, Vec<Vec<String>>) {
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
    let header = rows.remove(0);
    (header, rows)
}

#[test]
fn address_lists_match_the_full_database_in_both_formats() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = split(dir.path(), &with_unlinked(fixture()));
    let full = dir.path().join("full.sqlite");
    import(&inputs, &options(full.clone(), None)).unwrap();
    let csv = dir.path().join("adreslijst.csv");
    let stats = import(&inputs, &options(csv.clone(), Some(Format::Csv))).unwrap();
    assert_eq!(stats.rows, 5);
    let list = dir.path().join("adreslijst.sqlite");
    import(&inputs, &options(list.clone(), Some(Format::Sqlite))).unwrap();

    let (header, from_csv) = csv_rows(&csv);
    assert_eq!(header, ADDRESS_LIST.column_names().collect::<Vec<_>>());
    assert_eq!(from_csv, sqlite_rows(&list));

    // The same rows, derived from the full database's view.
    let db = Connection::open(&full).unwrap();
    let mut statement = db
        .prepare(
            "SELECT nummeraanduiding_identificatie, openbare_ruimte_naam, huisnummer, huisletter,
                    huisnummertoevoeging, postcode, woonplaats_naam, gemeente_naam, provincie_naam,
                    type_adresseerbaar_object, breedtegraad, lengtegraad,
                    openbare_ruimte_identificatie
             FROM adressen
             WHERE nummeraanduiding_status != 'Naamgeving ingetrokken'
             ORDER BY nummeraanduiding_identificatie",
        )
        .unwrap();
    let expected: Vec<Vec<Value>> = statement
        .query_map([], |row| {
            (0..ADDRESS_LIST.columns.len())
                .map(|i| row.get(i))
                .collect()
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let list_db = Connection::open(&list).unwrap();
    let actual: Vec<Vec<Value>> = list_db
        .prepare("SELECT * FROM adreslijst ORDER BY nummeraanduiding_identificatie")
        .unwrap()
        .query_map([], |row| {
            (0..ADDRESS_LIST.columns.len())
                .map(|i| row.get(i))
                .collect()
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(actual, expected);

    let unlinked = from_csv
        .iter()
        .find(|r| r[0] == "0000200000000009")
        .unwrap();
    assert_eq!(unlinked[1], "Hof, 't \"Zand\"");
    assert_eq!(
        &unlinked[6..],
        [
            "Andere stad",
            "Groningen",
            "Groningen",
            "",
            "",
            "",
            "0000300000000002"
        ]
    );
    let main = from_csv
        .iter()
        .find(|r| r[0] == "0000200000000001")
        .unwrap();
    assert_eq!(
        &main[6..12],
        [
            "'s-Test & Stad",
            "Nuenen, Gerwen en Nederwetten",
            "Noord-Brabant",
            "Verblijfsobject",
            "52.2117337",
            "5.9624422"
        ]
    );
    // Withdrawn and non-current addresses are left out.
    assert!(!from_csv.iter().any(|r| r[0] == "0000200000000004"));
    assert!(!from_csv.iter().any(|r| r[0] == "0000200000000005"));
}

#[test]
fn addresses_mixed_with_other_records_resolve_after_the_first_pass() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("bag.xml");
    fs::write(&input, with_unlinked(fixture())).unwrap();
    let split_inputs = split(dir.path(), &with_unlinked(fixture()));
    let mixed = dir.path().join("mixed.csv");
    import(&[input], &options(mixed.clone(), Some(Format::Csv))).unwrap();
    let separate = dir.path().join("separate.csv");
    import(
        &split_inputs[1..],
        &options(separate.clone(), Some(Format::Csv)),
    )
    .unwrap();
    assert_eq!(csv_rows(&mixed), csv_rows(&separate));
}

#[test]
fn address_list_lookups_answer_addresses_but_not_nearest() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = split(dir.path(), &fixture());
    let list = dir.path().join("adreslijst.sqlite");
    import(&inputs, &options(list.clone(), Some(Format::Sqlite))).unwrap();
    let db = query::open(&list).unwrap();
    let response = api::get(
        &db,
        "/v1/adressen?postcode=7311KZ&huisnummer=110&huisletter=A",
    )
    .unwrap();
    assert_eq!(response.status, 200);
    let body: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    let results = &body["adressen"];
    let ids: Vec<_> = results
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["nummeraanduidingIdentificatie"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["0000200000000001", "0000200000000002"]);
    assert_eq!(results[0]["gemeenteNaam"], "Nuenen, Gerwen en Nederwetten");
    // An address list has no object identifiers, statuses or buildings to return.
    assert_eq!(
        results[0]["adresseerbaarObject"],
        serde_json::json!({
            "type": "Verblijfsobject",
            "geometrie": { "type": "Point", "coordinates": [5.96244217, 52.21173368] }
        })
    );
    assert!(results[0].get("status").is_none());
    let response = api::get(&db, "/v1/adressen/0000200000000002").unwrap();
    assert_eq!(response.status, 200);
    let response = api::get(&db, "/v1/adressen/_dichtstbij?punt=5.9,52.2").unwrap();
    assert_eq!(response.status, 501);
    assert!(
        response.body.contains("vereist een database met tabellen"),
        "{}",
        response.body
    );
}

#[test]
fn address_lists_fail_on_inconsistent_data_without_publishing() {
    let dir = tempfile::tempdir().unwrap();
    for (edit, message) in [
        (
            (
                "<Objecten-ref:NummeraanduidingRef>0000200000000008</Objecten-ref:NummeraanduidingRef>",
                "<Objecten-ref:NummeraanduidingRef>0000200000000077</Objecten-ref:NummeraanduidingRef>",
            ),
            "1 links from adresseerbare objecten refer to missing nummeraanduidingen, for example 0000200000000077",
        ),
        (
            (
                "<Objecten:huisnummer>112</Objecten:huisnummer><Objecten:postcode>7311KZ</Objecten:postcode><Objecten:status>Naamgeving uitgegeven</Objecten:status><Objecten:ligtAan><Objecten-ref:OpenbareRuimteRef>0000300000000001",
                "<Objecten:huisnummer>112</Objecten:huisnummer><Objecten:postcode>7311KZ</Objecten:postcode><Objecten:status>Naamgeving uitgegeven</Objecten:status><Objecten:ligtAan><Objecten-ref:OpenbareRuimteRef>0000300000000099",
            ),
            "nummeraanduidingen refer to missing openbare ruimten, for example 0000300000000099",
        ),
        (
            (
                "<Objecten-ref:NummeraanduidingRef>0000200000000003</Objecten-ref:NummeraanduidingRef></Objecten:heeftAlsHoofdadres>",
                "<Objecten-ref:NummeraanduidingRef>0000200000000001</Objecten-ref:NummeraanduidingRef></Objecten:heeftAlsHoofdadres>",
            ),
            "1 nummeraanduidingen belong to more than one adresseerbaar object, for example 0000200000000001",
        ),
        (
            (
                "<Objecten:identificatie>0000200000000002</Objecten:identificatie>",
                "<Objecten:identificatie>0000200000000001</Objecten:identificatie>",
            ),
            "duplicate Nummeraanduiding identificatie 0000200000000001",
        ),
    ] {
        let xml = fixture();
        assert!(xml.contains(edit.0), "{}", edit.0);
        let inputs = split(dir.path(), &xml.replace(edit.0, edit.1));
        for format in [Format::Csv, Format::Sqlite] {
            let output = dir.path().join("list");
            let error = import(&inputs, &options(output.clone(), Some(format))).unwrap_err();
            assert!(format!("{error:#}").contains(message), "{error:#}");
            assert!(!output.exists());
        }
    }
    // Address files are read last, so they may hold nothing but addresses.
    let mut inputs = split(dir.path(), &fixture());
    let objects = inputs.remove(1);
    fs::rename(&objects, dir.path().join("9999NUM-objects.xml")).unwrap();
    inputs.insert(1, dir.path().join("9999NUM-objects.xml"));
    let error = import(
        &inputs,
        &options(dir.path().join("list.csv"), Some(Format::Csv)),
    )
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("may hold only nummeraanduidingen and panden"),
        "{error:#}"
    );
}
