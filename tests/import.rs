mod common;

use bagkit::{api, geo, geometry, query, xml};
use common::{ImportOptions, ImportStats, import};
use rusqlite::Connection;
use std::{
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bag.xml")
}
fn options(output: PathBuf) -> ImportOptions {
    ImportOptions {
        output,
        date: "2026-10-07".into(),
        force: false,
        cache_mib: 8,
        geometry: true,
        threads: 0,
        address_list: None,
    }
}
fn count(db: &Connection, table: &str) -> i64 {
    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
/// The addresses an API request returns.
fn lookup(db: &query::Database, target: &str) -> serde_json::Value {
    let response = api::get(db, target).unwrap();
    assert_eq!(response.status, 200, "{target}: {}", response.body);
    let mut body: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    body["adressen"].take()
}
fn coordinates(db: &query::Database, lat: f64, lon: f64) -> serde_json::Value {
    lookup(db, &format!("/v1/adressen/_dichtstbij?punt={lon},{lat}"))
}
fn ids(results: &serde_json::Value) -> Vec<&str> {
    results
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["nummeraanduidingIdentificatie"].as_str().unwrap())
        .collect()
}
/// Imports the fixture after applying `edit` to its XML.
fn import_variant(dir: &Path, edit: impl FnOnce(String) -> String) -> anyhow::Result<ImportStats> {
    let input = dir.join("variant.xml");
    fs::write(&input, edit(fs::read_to_string(fixture()).unwrap())).unwrap();
    import(&[input], &options(dir.join("variant.sqlite")))
}
fn append(xml: String, records: &str) -> String {
    xml.replace(
        "</sl:standBestand>",
        &format!("{records}</sl:standBestand>"),
    )
}
/// The fixture's `<sl:stand>` record whose identifier is `id`.
fn record(xml: &str, id: &str) -> String {
    let at = xml
        .find(&format!(">{id}</Objecten:identificatie>"))
        .unwrap();
    let start = xml[..at].rfind("<sl:stand>").unwrap();
    let end = xml[at..].find("</sl:stand>").unwrap() + at + "</sl:stand>".len();
    xml[start..end].to_owned()
}

#[test]
fn imports_core_tables_relations_and_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let opts = options(dir.path().join("bag.sqlite"));
    let stats = import(&[fixture()], &opts).unwrap();
    assert_eq!(stats.records, 16);
    assert_eq!(stats.current, 13);
    let db = query::open(&opts.output).unwrap();
    for (table, expected) in [
        ("woonplaatsen", 2),
        ("openbare_ruimten", 1),
        ("nummeraanduidingen", 5),
        ("adresseerbare_objecten", 3),
        ("panden", 2),
        ("verblijfsobject_panden", 2),
        ("adresseerbare_objecten WHERE breedtegraad IS NOT NULL", 3),
        ("adresseerbare_objecten_rtree", 3),
        ("panden WHERE geometrie IS NOT NULL", 2),
        ("adresseerbare_objecten WHERE vlak IS NOT NULL", 2),
        ("adressen", 5),
    ] {
        assert_eq!(count(&db, table), expected, "{table}");
    }
    // Identifiers are integers, and the official zero-padded text still finds them.
    let (object, secondary, locality, letter): (i64, bool, String, String) = db
        .query_row(
            "SELECT adresseerbaar_object_identificatie, indicatie_nevenadres, woonplaats_naam, huisletter
             FROM adressen
             WHERE nummeraanduiding_identificatie = '0000200000000002'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        (object, secondary, locality.as_str(), letter.as_str()),
        (10000000001, true, "Andere stad", "A")
    );
    let (short_name, purposes, area): (String, String, i64) = db
        .query_row(
            "SELECT verkorte_naam, gebruiksdoelen, oppervlakte
             FROM openbare_ruimten, adresseerbare_objecten
             WHERE adresseerbare_objecten.type = 'Verblijfsobject'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(short_name, "Éénstr.");
    assert_eq!(purposes, r#"["kantoorfunctie","woonfunctie"]"#);
    assert_eq!(area, 8870);
    // Withdrawn addresses keep their object link but are not returned by lookups.
    let withdrawn: Option<i64> = db
        .query_row(
            "SELECT adresseerbaar_object_identificatie
             FROM nummeraanduidingen
             WHERE identificatie = 200000000004",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(withdrawn, Some(10000000001));
    let results = lookup(
        &db,
        "/v1/adressen?postcode=7311KZ&huisnummer=110&huisletter=a&huisnummertoevoeging=2",
    );
    assert_eq!(ids(&results), ["0000200000000002"]);
    let object = &results[0]["adresseerbaarObject"];
    assert_eq!(results[0]["indicatieNevenadres"], true);
    assert_eq!(object["identificatie"], "0000010000000001");
    assert_eq!(object["type"], "Verblijfsobject");
    assert_eq!(
        object["gebruiksdoelen"],
        serde_json::json!(["kantoorfunctie", "woonfunctie"])
    );
    assert_eq!(object["panden"].as_array().unwrap().len(), 2);
    assert_eq!(object["panden"][1]["oorspronkelijkBouwjaar"], 1986);
    assert_eq!(
        object["geometrie"],
        serde_json::json!({ "type": "Point", "coordinates": [5.96244217, 52.21173368] })
    );
    let postcode = lookup(&db, "/v1/adressen?postcode=7311KZ");
    let mut all = ids(&postcode);
    all.sort_unstable();
    assert_eq!(
        all,
        [
            "0000200000000001",
            "0000200000000002",
            "0000200000000003",
            "0000200000000008"
        ]
    );
    assert_eq!(
        ids(&lookup(
            &db,
            "/v1/adressen?postcode=7311KZ&huisnummer=110&huisnummertoevoeging=2&exacteMatch=true"
        )),
        ["0000200000000003"]
    );
    let plan: String = db
        .query_row(
            "EXPLAIN QUERY PLAN
             SELECT *
             FROM nummeraanduidingen INDEXED BY nummeraanduidingen_by_postcode
             WHERE postcode = '7311KZ'
                 AND huisnummer = 110",
            [],
            |r| r.get(3),
        )
        .unwrap();
    assert!(plan.contains("nummeraanduidingen_by_postcode"), "{plan}");
    let metadata: Vec<(String, String)> = db
        .prepare("SELECT key, value FROM bagkit_metadata ORDER BY key")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(metadata.contains(&("geometry".into(), "included".into())));
    assert!(metadata.contains(&("reference_date".into(), "2026-10-07".into())));
    assert!(metadata.contains(&("schema_version".into(), bagkit::SCHEMA_VERSION.into())));
    let rtree_report: String = db
        .query_row(
            "SELECT rtreecheck('adresseerbare_objecten_rtree')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rtree_report, "ok");
    let foreign_key_problems = db
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query_map([], |_| Ok(()))
        .unwrap()
        .count();
    assert_eq!(foreign_key_problems, 0);
}

#[test]
fn footprints_are_registered_geopackage_polygons() {
    let dir = tempfile::tempdir().unwrap();
    let opts = options(dir.path().join("bag.sqlite"));
    import(&[fixture()], &opts).unwrap();
    let db = query::open(&opts.output).unwrap();
    let application_id: i64 = db
        .query_row("PRAGMA application_id", [], |r| r.get(0))
        .unwrap();
    assert_eq!(application_id, 0x4750_4B47);
    let registered: Vec<(String, String, i64)> = db
        .prepare(
            "SELECT table_name, geometry_type_name, gpkg_geometry_columns.srs_id
             FROM gpkg_geometry_columns
             JOIN gpkg_contents USING (table_name)
             ORDER BY table_name",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        registered,
        [
            ("adresseerbare_objecten".into(), "POLYGON".into(), 28992),
            ("panden".into(), "POLYGON".into(), 28992)
        ]
    );
    let blob: Vec<u8> = db
        .query_row(
            "SELECT geometrie FROM panden WHERE identificatie = 100000000001",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let f64_at = |at: usize| f64::from_le_bytes(blob[at..at + 8].try_into().unwrap());
    let u32_at = |at: usize| u32::from_le_bytes(blob[at..at + 4].try_into().unwrap());
    assert_eq!(&blob[..4], b"GP\0\x03");
    assert_eq!(i32::from_le_bytes(blob[4..8].try_into().unwrap()), 28992);
    assert_eq!(
        [f64_at(8), f64_at(16), f64_at(24), f64_at(32)],
        [155000.0, 155100.0, 463000.0, 463100.0]
    );
    assert_eq!(blob[40], 1);
    assert_eq!([u32_at(41), u32_at(45), u32_at(49)], [3, 1, 5]);
    assert_eq!([f64_at(53), f64_at(61)], [155000.0, 463000.0]);
    assert_eq!(blob.len(), 53 + 5 * 16);
}

#[test]
fn import_without_geometry_keeps_points_and_nearest_lookup() {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = options(dir.path().join("bag.sqlite"));
    opts.geometry = false;
    import(&[fixture()], &opts).unwrap();
    let db = query::open(&opts.output).unwrap();
    let tables: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name LIKE 'gpkg%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0);
    assert_eq!(count(&db, "panden WHERE geometrie IS NOT NULL"), 0);
    assert_eq!(
        count(&db, "adresseerbare_objecten WHERE vlak IS NOT NULL"),
        0
    );
    let application_id: i64 = db
        .query_row("PRAGMA application_id", [], |r| r.get(0))
        .unwrap();
    assert_eq!(application_id, 0);
    assert_eq!(
        count(&db, "adresseerbare_objecten WHERE breedtegraad IS NOT NULL"),
        3
    );
    let (lat, lon) = geo::rd_to_wgs84(155050.0, 463050.0);
    assert_eq!(ids(&coordinates(&db, lat, lon)), ["0000200000000008"]);
}

#[test]
fn nearest_lookup_measures_meters_and_widens_its_search() {
    let dir = tempfile::tempdir().unwrap();
    let opts = options(dir.path().join("bag.sqlite"));
    import(&[fixture()], &opts).unwrap();
    let db = query::open(&opts.output).unwrap();
    // The main address comes first; the withdrawn secondary address is left out.
    let results = coordinates(&db, 52.2117336828881, 5.9624421719591);
    assert_eq!(ids(&results), ["0000200000000001", "0000200000000002"]);
    assert_eq!(results[0]["indicatieNevenadres"], false);
    assert_eq!(results[0]["afstand"], 0.0);
    // One kilometre east of the unit, well beyond the first 100 m search box.
    let (lat, lon) = geo::rd_to_wgs84(195315.783, 469449.074);
    let results = coordinates(&db, lat, lon);
    assert_eq!(ids(&results)[0], "0000200000000001");
    assert!((results[0]["afstand"].as_f64().unwrap() - 1000.0).abs() < 0.002);
    // The mooring and pitch share a centroid; the smaller identifier (the mooring) wins.
    // Moorings have no building but are still found.
    let (lat, lon) = geo::rd_to_wgs84(155050.0, 463050.0);
    let results = coordinates(&db, lat, lon);
    assert_eq!(ids(&results), ["0000200000000008"]);
    assert_eq!(results[0]["adresseerbaarObject"]["type"], "Ligplaats");
    assert_eq!(
        results[0]["adresseerbaarObject"]["panden"],
        serde_json::json!([])
    );
    assert_eq!(coordinates(&db, 0.0, 0.0), serde_json::json!([]));
    assert_eq!(coordinates(&db, 53.9, 7.9), serde_json::json!([]));
}

#[test]
fn placeholder_coordinates_outside_rd_new_get_no_wgs84_point() {
    let dir = tempfile::tempdir().unwrap();
    let stats = import_variant(dir.path(), |xml| {
        xml.replace("194315.783 469449.074 0", "0.0 0.0 0.0")
    })
    .unwrap();
    assert_eq!(stats.points_outside_rd_new, 1);
    let db = query::open(&dir.path().join("variant.sqlite")).unwrap();
    let (x, lat): (f64, Option<f64>) = db
        .query_row(
            "SELECT rd_x, breedtegraad FROM adresseerbare_objecten WHERE type = 'Verblijfsobject'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((x, lat), (0.0, None));
    assert_eq!(
        count(&db, "adresseerbare_objecten WHERE breedtegraad IS NOT NULL"),
        2
    );
    assert_eq!(count(&db, "adresseerbare_objecten_rtree"), 2);
    let results = lookup(
        &db,
        "/v1/adressen?postcode=7311KZ&huisnummer=110&huisletter=a&huisnummertoevoeging=2",
    );
    assert_eq!(
        results[0]["adresseerbaarObject"]["geometrie"],
        serde_json::Value::Null
    );
}

#[test]
fn objects_with_only_withdrawn_addresses_are_not_nearest_candidates() {
    let dir = tempfile::tempdir().unwrap();
    import_variant(dir.path(), |xml| {
        let address = record(&xml, "0000200000000008");
        xml.replace(
            &address,
            &address.replace("Naamgeving uitgegeven", "Naamgeving ingetrokken"),
        )
    })
    .unwrap();
    let db = query::open(&dir.path().join("variant.sqlite")).unwrap();
    assert_eq!(count(&db, "adresseerbare_objecten_rtree"), 2);
    let (lat, lon) = geo::rd_to_wgs84(155050.0, 463050.0);
    assert_eq!(ids(&coordinates(&db, lat, lon)), ["0000200000000003"]);
}

#[test]
fn resolves_references_regardless_of_input_order() {
    let dir = tempfile::tempdir().unwrap();
    let xml = fs::read_to_string(fixture()).unwrap();
    let header = &xml[..xml.find("<sl:stand>").unwrap()];
    let mut objects: Vec<_> = xml
        .split("<sl:stand>")
        .skip(1)
        .map(|s| {
            format!(
                "<sl:stand>{}",
                s.split("</sl:stand>").next().unwrap().to_owned() + "</sl:stand>"
            )
        })
        .collect();
    objects.reverse();
    let input = dir.path().join("reverse.xml");
    fs::write(
        &input,
        format!(
            "{header}{}</sl:standBestand></sl-bag-extract:bagStand>",
            objects.join("")
        ),
    )
    .unwrap();
    let opts = options(dir.path().join("bag.sqlite"));
    import(&[input], &opts).unwrap();
    let db = query::open(&opts.output).unwrap();
    assert_eq!(count(&db, "adressen"), 5);
}

#[test]
fn failures_preserve_existing_database_and_do_not_publish_partial_output() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("bad.xml");
    fs::write(&input, "<root><broken>").unwrap();
    let mut opts = options(dir.path().join("bag.sqlite"));
    assert!(import(std::slice::from_ref(&input), &opts).is_err());
    assert!(!opts.output.exists());
    fs::write(&opts.output, b"original").unwrap();
    opts.force = true;
    assert!(import(std::slice::from_ref(&input), &opts).is_err());
    assert_eq!(fs::read(&opts.output).unwrap(), b"original");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn nested_zip_and_gzip_match_plain_xml_and_record_the_extract_date() {
    use zip::{ZipWriter, write::SimpleFileOptions};
    let dir = tempfile::tempdir().unwrap();
    let source = fs::read_to_string(fixture()).unwrap().replacen(
        "<sl:standBestand>",
        "<sl-bag-extract:bagInfo><selecties-extract:LVC-Extract><selecties-extract:StandTechnischeDatum>2026-10-06</selecties-extract:StandTechnischeDatum></selecties-extract:LVC-Extract></sl-bag-extract:bagInfo><sl:standBestand>",
        1,
    );
    let mut inner = ZipWriter::new(Cursor::new(Vec::new()));
    inner
        .start_file(
            "bag.xml",
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
    inner.write_all(source.as_bytes()).unwrap();
    let inner = inner.finish().unwrap().into_inner();
    let zip = dir.path().join("national.zip");
    let mut outer = ZipWriter::new(fs::File::create(&zip).unwrap());
    outer
        .start_file("../extract.zip", SimpleFileOptions::default())
        .unwrap();
    outer.write_all(&inner).unwrap();
    outer.finish().unwrap();
    let gzip = dir.path().join("bag.xml.gz");
    let mut encoder = flate2::write::GzEncoder::new(
        fs::File::create(&gzip).unwrap(),
        flate2::Compression::default(),
    );
    encoder.write_all(source.as_bytes()).unwrap();
    encoder.finish().unwrap();
    for (i, input) in [zip, gzip].into_iter().enumerate() {
        let opts = options(dir.path().join(format!("{i}.sqlite")));
        let stats = import(&[input], &opts).unwrap();
        assert_eq!(stats.current, 13);
        let db = query::open(&opts.output).unwrap();
        assert_eq!(count(&db, "adressen"), 5);
        let extract_date: String = db
            .query_row(
                "SELECT value FROM bagkit_metadata WHERE key = 'extract_date'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(extract_date, "2026-10-06");
    }
}

#[test]
fn validity_is_start_inclusive_and_end_exclusive() {
    let v = xml::Validity {
        has_history: true,
        begin: "2026-10-07".into(),
        end: "2026-10-08".into(),
        ..Default::default()
    };
    assert!(!v.is_current("2026-10-06").unwrap());
    assert!(v.is_current("2026-10-07").unwrap());
    assert!(!v.is_current("2026-10-08").unwrap());
    assert!(xml::validate_date("2026-02-30").is_err());
}

#[test]
fn corrected_and_non_bag_records_are_excluded() {
    let dir = tempfile::tempdir().unwrap();
    let marked = |id: &str, postcode: &str, marker: &str| {
        format!(
            "<sl:stand><sl-bag-extract:bagObject><Objecten:Nummeraanduiding><Objecten:identificatie>{id}</Objecten:identificatie><Objecten:huisnummer>1</Objecten:huisnummer><Objecten:postcode>{postcode}</Objecten:postcode><Objecten:status>Naamgeving uitgegeven</Objecten:status><Objecten:ligtAan><Objecten-ref:OpenbareRuimteRef>0000300000000001</Objecten-ref:OpenbareRuimteRef></Objecten:ligtAan><Objecten:voorkomen><Historie:Voorkomen><Historie:beginGeldigheid>2000-01-01</Historie:beginGeldigheid><Historie:{marker}>2001-01-01T00:00:00.000</Historie:{marker}></Historie:Voorkomen></Objecten:voorkomen></Objecten:Nummeraanduiding></sl-bag-extract:bagObject></sl:stand>"
        )
    };
    let stats = import_variant(dir.path(), |xml| {
        append(
            xml,
            &(marked("0000200000000001", "9999ZZ", "tijdstipInactief")
                + &marked("0000200000000099", "9998ZZ", "tijdstipNietBagLV")),
        )
    })
    .unwrap();
    assert_eq!(stats.records, 18);
    assert_eq!(stats.current, 13);
    let db = query::open(&dir.path().join("variant.sqlite")).unwrap();
    assert_eq!(count(&db, "nummeraanduidingen"), 5);
    for path in [
        "/v1/adressen?postcode=9999ZZ",
        "/v1/adressen?postcode=9998ZZ",
    ] {
        assert_eq!(lookup(&db, path), serde_json::json!([]));
    }
}

#[test]
fn import_rejects_duplicate_and_malformed_identifiers() {
    let dir = tempfile::tempdir().unwrap();
    let error = import_variant(dir.path(), |xml| {
        let unit = record(&xml, "0000010000000001");
        append(xml, &unit)
    })
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("duplicate Verblijfsobject identificatie 0000010000000001"),
        "{error:#}"
    );
    let error = import_variant(dir.path(), |xml| {
        xml.replace(
            ">0000100000000002</Objecten:identificatie>",
            ">12</Objecten:identificatie>",
        )
    })
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("identificatie '12' is not 16 digits"),
        "{error:#}"
    );
}

#[test]
fn duplicates_are_found_across_insert_batches() {
    let dir = tempfile::tempdir().unwrap();
    let address = |id: u64, number: u64| {
        format!(
            "<sl:stand><sl-bag-extract:bagObject><Objecten:Nummeraanduiding><Objecten:identificatie>{id:016}</Objecten:identificatie><Objecten:huisnummer>{number}</Objecten:huisnummer><Objecten:postcode>1234AB</Objecten:postcode><Objecten:status>Naamgeving uitgegeven</Objecten:status><Objecten:ligtAan><Objecten-ref:OpenbareRuimteRef>0000300000000001</Objecten-ref:OpenbareRuimteRef></Objecten:ligtAan><Objecten:voorkomen><Historie:Voorkomen><Historie:beginGeldigheid>2000-01-01</Historie:beginGeldigheid></Historie:Voorkomen></Objecten:voorkomen></Objecten:Nummeraanduiding></sl-bag-extract:bagObject></sl:stand>"
        )
    };
    let many: String = (0..300).map(|i| address(200000001000 + i, i + 1)).collect();
    let stats = import_variant(dir.path(), |xml| append(xml, &many)).unwrap();
    assert_eq!(stats.current, 313);
    let db = query::open(&dir.path().join("variant.sqlite")).unwrap();
    assert_eq!(
        lookup(&db, "/v1/adressen?postcode=1234AB")
            .as_array()
            .unwrap()
            .len(),
        300
    );
    // The same address again, about 200 records later and inside a full batch, with a
    // different house number.
    let other = tempfile::tempdir().unwrap();
    let repeated: String = (0..200)
        .map(|i| address(200000001000 + i, i + 1))
        .chain([address(200000001003, 999)])
        .chain((200..300).map(|i| address(200000001000 + i, i + 1)))
        .collect();
    let error = import_variant(other.path(), |xml| append(xml, &repeated)).unwrap_err();
    assert!(
        format!("{error:#}").contains("duplicate Nummeraanduiding identificatie 0000200000001003"),
        "{error:#}"
    );
}

/// The fixture with its addresses after all other records, as in the national extract's
/// processing order, so the writer resolves links from memory.
fn objects_before_addresses(xml: String) -> String {
    let start = xml.find("<sl:stand>").unwrap();
    let end = xml.find("</sl:standBestand>").unwrap();
    let records: Vec<String> = xml[start..end]
        .split_inclusive("</sl:stand>")
        .map(|r| r.trim().to_owned())
        .filter(|r| !r.is_empty())
        .collect();
    let (addresses, others): (Vec<_>, Vec<_>) = records
        .into_iter()
        .partition(|r| r.contains("<Objecten:Nummeraanduiding>"));
    format!(
        "{}{}{}{}",
        &xml[..start],
        others.concat(),
        addresses.concat(),
        &xml[end..]
    )
}

#[test]
fn links_resolve_the_same_whether_objects_or_addresses_come_first() {
    let snapshot = |db: &query::Database| -> Vec<(i64, Option<i64>, Option<bool>)> {
        db.prepare(
            "SELECT identificatie, adresseerbaar_object_identificatie, indicatie_nevenadres
             FROM nummeraanduidingen
             ORDER BY identificatie",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    };
    let dir = tempfile::tempdir().unwrap();
    import(&[fixture()], &options(dir.path().join("bag.sqlite"))).unwrap();
    let addresses_first = snapshot(&query::open(&dir.path().join("bag.sqlite")).unwrap());
    import_variant(dir.path(), objects_before_addresses).unwrap();
    let objects_first = snapshot(&query::open(&dir.path().join("variant.sqlite")).unwrap());
    assert_eq!(objects_first, addresses_first);
    assert!(objects_first.contains(&(200000000002, Some(10000000001), Some(true))));
}

#[test]
fn links_from_memory_still_reject_duplicates_and_missing_addresses() {
    let dir = tempfile::tempdir().unwrap();
    let error = import_variant(dir.path(), |xml| {
        objects_before_addresses(xml.replace(
            "<Objecten-ref:NummeraanduidingRef>0000200000000008</Objecten-ref:NummeraanduidingRef>",
            "<Objecten-ref:NummeraanduidingRef>0000200000000003</Objecten-ref:NummeraanduidingRef>",
        ))
    })
    .unwrap_err();
    assert!(
        format!("{error:#}").contains(
            "1 nummeraanduidingen belong to more than one adresseerbaar object, for example 0000200000000003"
        ),
        "{error:#}"
    );
    let other = tempfile::tempdir().unwrap();
    let error = import_variant(other.path(), |xml| {
        objects_before_addresses(xml.replace(
            "<Objecten-ref:NummeraanduidingRef>0000200000000008</Objecten-ref:NummeraanduidingRef>",
            "<Objecten-ref:NummeraanduidingRef>0000200000000077</Objecten-ref:NummeraanduidingRef>",
        ))
    })
    .unwrap_err();
    assert!(
        format!("{error:#}").contains(
            "1 links from adresseerbare objecten refer to missing nummeraanduidingen, for example 0000200000000077"
        ),
        "{error:#}"
    );
}

#[test]
fn import_rejects_inconsistent_address_links() {
    let dir = tempfile::tempdir().unwrap();
    // The mooring claims the pitch's main address.
    let error = import_variant(dir.path(), |xml| {
        xml.replace(
            "<Objecten-ref:NummeraanduidingRef>0000200000000008</Objecten-ref:NummeraanduidingRef>",
            "<Objecten-ref:NummeraanduidingRef>0000200000000003</Objecten-ref:NummeraanduidingRef>",
        )
    })
    .unwrap_err();
    assert!(
        format!("{error:#}").contains(
            "1 nummeraanduidingen belong to more than one adresseerbaar object, for example 0000200000000003"
        ),
        "{error:#}"
    );
    // The pitch's main link becomes a secondary one.
    let error = import_variant(dir.path(), |xml| {
        let pitch = record(&xml, "0000030000000001");
        xml.replace(
            &pitch,
            &pitch.replace("heeftAlsHoofdadres", "heeftAlsNevenadres"),
        )
    })
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("Standplaats 0000030000000001 has 0 hoofdadressen"),
        "{error:#}"
    );
}

#[test]
fn missing_references_and_existing_outputs_fail_without_replacing_data() {
    let dir = tempfile::tempdir().unwrap();
    for (edit, message) in [
        (
            (
                "<Objecten-ref:PandRef>0000100000000002</Objecten-ref:PandRef>",
                "<Objecten-ref:PandRef>0000100000000009</Objecten-ref:PandRef>",
            ),
            "links from verblijfsobjecten refer to missing panden, for example 0000100000000009",
        ),
        (
            (
                "<Objecten-ref:NummeraanduidingRef>0000200000000008</Objecten-ref:NummeraanduidingRef>",
                "<Objecten-ref:NummeraanduidingRef>0000200000000077</Objecten-ref:NummeraanduidingRef>",
            ),
            "links from adresseerbare objecten refer to missing nummeraanduidingen, for example 0000200000000077",
        ),
        (
            (
                "<Objecten-ref:WoonplaatsRef>5678</Objecten-ref:WoonplaatsRef></Objecten:ligtIn>",
                "<Objecten-ref:WoonplaatsRef>9999</Objecten-ref:WoonplaatsRef></Objecten:ligtIn>",
            ),
            "nummeraanduidingen refer to missing woonplaatsen, for example 9999",
        ),
    ] {
        let error = import_variant(dir.path(), |xml| xml.replace(edit.0, edit.1)).unwrap_err();
        assert!(format!("{error:#}").contains(message), "{error:#}");
    }
    let input = dir.path().join("incomplete.xml");
    fs::write(&input, "<bagObject><Nummeraanduiding><identificatie>0000200000000001</identificatie><huisnummer>1</huisnummer><status>Naamgeving uitgegeven</status><ligtAan><OpenbareRuimteRef>0000300000000002</OpenbareRuimteRef></ligtAan></Nummeraanduiding></bagObject>").unwrap();
    let opts = options(dir.path().join("bag.sqlite"));
    let error = import(&[input], &opts).unwrap_err();
    assert!(
        format!("{error:#}").contains("nummeraanduidingen refer to missing openbare ruimten"),
        "{error:#}"
    );
    assert!(!opts.output.exists());
    fs::write(&opts.output, b"keep").unwrap();
    assert!(
        import(&[fixture()], &opts)
            .unwrap_err()
            .to_string()
            .contains("--force")
    );
    assert_eq!(fs::read(&opts.output).unwrap(), b"keep");
}

#[test]
fn namespaces_entities_and_cdata_are_supported() {
    let data = br#"<x:bagObject xmlns:x="urn:x"><b:Woonplaats xmlns:b="urn:b"><b:identificatie>1234</b:identificatie><b:naam>A &amp; B<![CDATA[ <C>]]> &#233;</b:naam><b:status>Woonplaats aangewezen</b:status></b:Woonplaats></x:bagObject>"#;
    let mut objects = Vec::new();
    xml::parse(Cursor::new(data), |r| {
        objects.push(r.into_object(true)?);
        Ok(())
    })
    .unwrap();
    assert!(matches!(&objects[0], xml::BagObject::Locality(l) if l.name == "A & B <C> é"));
}

#[test]
fn malformed_xml_and_missing_validity_start_are_rejected() {
    assert!(
        xml::parse(Cursor::new("<bagObject><Woonplaats></bagObject>"), |_| Ok(
            ()
        ))
        .is_err()
    );
    assert!(xml::parse(Cursor::new("<!DOCTYPE bagObject><bagObject/>"), |_| Ok(())).is_err());
    let v = xml::Validity {
        has_history: true,
        ..Default::default()
    };
    assert!(v.is_current("2026-10-07").is_err());
}

fn parse_object(data: &str) -> xml::BagObject {
    let mut objects = Vec::new();
    xml::parse(Cursor::new(data), |r| {
        objects.push(r.into_object(true)?);
        Ok(())
    })
    .unwrap();
    objects.pop().unwrap()
}

#[test]
fn polygons_use_count_or_dimension_and_keep_interior_rings() {
    // Three-dimensional ring declared on the polygon; heights are dropped.
    let object = parse_object(
        r#"<bagObject><Standplaats><identificatie>0000030000000001</identificatie><status>Plaats aangewezen</status><heeftAlsHoofdadres><NummeraanduidingRef>0000200000000001</NummeraanduidingRef></heeftAlsHoofdadres><geometrie><Polygon srsDimension="3"><exterior><LinearRing><posList>
155000 463000 0 155100 463000 8 155100 463100 7 155000 463000 0
</posList></LinearRing></exterior></Polygon></geometrie></Standplaats></bagObject>"#,
    );
    let xml::BagObject::AddressableObject(pitch) = object else {
        panic!("expected an addressable object")
    };
    let Some(xml::Geometry::Polygon(rings)) = pitch.geometry else {
        panic!("expected a polygon")
    };
    assert_eq!(rings[0][1], [155100.0, 463000.0]);
    // `count` wins over a missing or wrong dimension; an interior ring is a hole.
    let object = parse_object(
        r#"<bagObject><Pand><identificatie>0000100000000001</identificatie><status>Pand in gebruik</status><oorspronkelijkBouwjaar>1900</oorspronkelijkBouwjaar><geometrie><Polygon><exterior><LinearRing><posList count="5">0 0 0 4 0 0 4 4 0 0 4 0 0 0 0</posList></LinearRing></exterior><interior><LinearRing><posList count="5">0 0 0 0 2 0 2 2 0 2 0 0 0 0 0</posList></LinearRing></interior></Polygon></geometrie></Pand></bagObject>"#,
    );
    let xml::BagObject::Building(building) = object else {
        panic!("expected a building")
    };
    let rings = building.footprint.unwrap();
    assert_eq!(rings.len(), 2);
    // A 4x4 square minus its lower-left 2x2 quarter.
    let [x, y] = geometry::centroid(&rings);
    assert!((x - 7.0 / 3.0).abs() < 1e-12 && (y - 7.0 / 3.0).abs() < 1e-12);
}

#[test]
fn degenerate_polygons_fall_back_to_their_vertex_mean() {
    let line = vec![vec![[0.0, 0.0], [2.0, 0.0], [4.0, 0.0], [0.0, 0.0]]];
    assert_eq!(geometry::centroid(&line), [2.0, 0.0]);
}

#[test]
fn rd_conversion_round_trips_across_the_netherlands() {
    for (x, y) in [
        (155000.0, 463000.0),
        (13000.0, 370000.0),
        (280000.0, 610000.0),
        (190000.0, 310000.0),
        (90000.0, 590000.0),
    ] {
        let (lat, lon) = geo::rd_to_wgs84(x, y);
        let (rx, ry) = geo::wgs84_to_rd(lat, lon).unwrap();
        assert!((rx - x).abs() < 1e-5 && (ry - y).abs() < 1e-5, "{x} {y}");
    }
    assert!(geo::wgs84_to_rd(0.0, 0.0).is_none());
}

#[test]
fn successful_replacement_publishes_a_new_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = options(dir.path().join("bag.sqlite"));
    import(&[fixture()], &opts).unwrap();
    assert_eq!(
        count(&query::open(&opts.output).unwrap(), "nummeraanduidingen"),
        5
    );
    opts.force = true;
    opts.date = "2026-10-08".into();
    import(&[fixture()], &opts).unwrap();
    assert_eq!(
        count(&query::open(&opts.output).unwrap(), "nummeraanduidingen"),
        6
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn replacement_refuses_a_database_with_wal_sidecars() {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = options(dir.path().join("bag.sqlite"));
    fs::write(&opts.output, b"existing database").unwrap();
    fs::write(dir.path().join("bag.sqlite-wal"), b"pending writes").unwrap();
    opts.force = true;
    assert!(
        import(&[fixture()], &opts)
            .unwrap_err()
            .to_string()
            .contains("sidecar")
    );
    assert_eq!(fs::read(&opts.output).unwrap(), b"existing database");
}

#[test]
fn opening_a_database_from_another_schema_fails_clearly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("other.sqlite");
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE nums (id TEXT);")
        .unwrap();
    let error = query::open(&path).err().unwrap();
    assert!(
        error.to_string().contains("is not a bagkit database"),
        "{error:#}"
    );
}

/// A GEM-WPL-RELATIE document relating each locality to a municipality from `begin`, until
/// `end` when given.
fn municipality_relations(relations: &[(&str, &str, &str, Option<&str>)]) -> String {
    let records: String = relations
        .iter()
        .map(|(locality, municipality, begin, end)| {
            let end = end.map_or(String::new(), |end| {
                format!("<bagtypes:einddatumTijdvakGeldigheid>{end}</bagtypes:einddatumTijdvakGeldigheid>")
            });
            format!(
                "<gwr-product:GemeenteWoonplaatsRelatie><gwr-product:tijdvakgeldigheid>\
                 <bagtypes:begindatumTijdvakGeldigheid>{begin}</bagtypes:begindatumTijdvakGeldigheid>{end}\
                 </gwr-product:tijdvakgeldigheid><gwr-product:gerelateerdeWoonplaats>\
                 <gwr-product:identificatie>{locality}</gwr-product:identificatie></gwr-product:gerelateerdeWoonplaats>\
                 <gwr-product:gerelateerdeGemeente><gwr-product:identificatie>{municipality}</gwr-product:identificatie>\
                 </gwr-product:gerelateerdeGemeente><gwr-product:status>definitief</gwr-product:status>\
                 </gwr-product:GemeenteWoonplaatsRelatie>"
            )
        })
        .collect();
    format!(
        "<gwr-bestand:BAG-GWR-Deelbestand-LVC xmlns:gwr-bestand=\"urn:b\" xmlns:gwr-product=\"urn:p\" \
         xmlns:bagtypes=\"urn:t\"><gwr-bestand:Product><gwr-product:GemeenteWoonplaatsRelatieProduct>\
         {records}</gwr-product:GemeenteWoonplaatsRelatieProduct></gwr-bestand:Product>\
         </gwr-bestand:BAG-GWR-Deelbestand-LVC>"
    )
}

#[test]
fn localities_take_their_municipality_and_province_on_the_reference_date() {
    let dir = tempfile::tempdir().unwrap();
    let relations = dir.path().join("GEM-WPL-RELATIE.xml");
    fs::write(
        &relations,
        municipality_relations(&[
            ("1234", "0014", "2000-01-01", Some("2020-01-01")),
            ("1234", "0363", "2020-01-01", None),
            ("5678", "0014", "2000-01-01", None),
            // Not a current locality: ignored.
            ("4321", "0014", "2000-01-01", None),
        ]),
    )
    .unwrap();
    let opts = options(dir.path().join("bag.sqlite"));
    import(&[relations.clone(), fixture()], &opts).unwrap();
    let db = query::open(&opts.output).unwrap();
    let results = lookup(&db, "/v1/adressen?postcode=7311KZ&huisnummer=110");
    let place = |id: &str| {
        let r = results
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["nummeraanduidingIdentificatie"] == id)
            .unwrap();
        (
            r["woonplaatsNaam"].clone(),
            r["gemeenteNaam"].clone(),
            r["provincieNaam"].clone(),
        )
    };
    assert_eq!(
        place("0000200000000001"),
        (
            "'s-Test & Stad".into(),
            "Amsterdam".into(),
            "Noord-Holland".into()
        )
    );
    // The address's own locality overrides its public space's.
    assert_eq!(
        place("0000200000000002"),
        ("Andere stad".into(), "Groningen".into(), "Groningen".into())
    );
    assert_eq!(count(&db, "gemeenten"), 342);

    // Without GEM-WPL-RELATIE municipalities stay empty.
    let opts = options(dir.path().join("without.sqlite"));
    import(&[fixture()], &opts).unwrap();
    let db = query::open(&opts.output).unwrap();
    assert!(
        lookup(&db, "/v1/adressen?postcode=7311KZ&huisnummer=110")[0]["gemeenteNaam"].is_null()
    );

    for (relations_xml, message) in [
        (
            municipality_relations(&[
                ("1234", "0014", "2000-01-01", None),
                ("1234", "0363", "2000-01-01", None),
                ("5678", "0014", "2000-01-01", None),
            ]),
            "Woonplaats 1234 belongs to more than one gemeente",
        ),
        (
            municipality_relations(&[("1234", "0014", "2000-01-01", None)]),
            "1 woonplaatsen have no gemeente in GEM-WPL-RELATIE, for example 5678",
        ),
    ] {
        fs::write(&relations, relations_xml).unwrap();
        let error = import(
            &[relations.clone(), fixture()],
            &options(dir.path().join("x.sqlite")),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains(message), "{error:#}");
    }
}

type Emit<'a> = &'a mut dyn FnMut(xml::Record) -> anyhow::Result<()>;

/// Every record of `data` as debug text, then the extract date, or the error.
fn records_of(
    parse: impl FnOnce(Emit) -> anyhow::Result<xml::ParseSummary>,
) -> Result<Vec<String>, String> {
    let mut records = Vec::new();
    parse(&mut |record| {
        records.push(format!("{:?}", record.into_object(true)?));
        Ok(())
    })
    .map(|summary| {
        records.push(format!("{:?}", summary.extract_date));
        records
    })
    .map_err(|e| format!("{e:#}"))
}

/// What the streaming parser and the in-memory scanner make of `data`.
fn both_parsers(data: &[u8]) -> [Result<Vec<String>, String>; 2] {
    [
        records_of(|emit| xml::parse(Cursor::new(data), emit)),
        records_of(|emit| xml::parse_bytes(data, None, emit)),
    ]
}

#[test]
fn scanner_and_streaming_parser_agree() {
    let fixture = fs::read(fixture()).unwrap();
    let edge_cases = br#"<?xml version="1.0" encoding="UTF-8"?>
<!-- a comment with <tags> -->
<r xmlns:x="urn:x"><x:StandTechnischeDatum>2026-10-08</x:StandTechnischeDatum>
<x:bagObject><x:Woonplaats note='a > b' other="it's"><x:identificatie>1234</x:identificatie>
<x:naam>A &amp; B<![CDATA[ <C>]]> &#233;&#xE9; line&#13;
end</x:naam><x:empty/><x:status >Woonplaats aangewezen</x:status ></x:Woonplaats></x:bagObject>
<x:bagObject><x:Ligplaats><x:identificatie>0000020000000001</x:identificatie><x:status>Plaats aangewezen</x:status>
<x:geometrie><gml:Polygon xmlns:gml="urn:g" srsDimension="3"><gml:exterior><gml:LinearRing><gml:posList count="5">0 0 1 10 0 1 10 10 1 0 10 1 0 0 1</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></x:geometrie>
<x:heeftAlsHoofdadres><x:NummeraanduidingRef>0000200000000001</x:NummeraanduidingRef></x:heeftAlsHoofdadres></x:Ligplaats></x:bagObject></r>"#;
    for data in [&fixture[..], &edge_cases[..]] {
        let [streamed, scanned] = both_parsers(data);
        assert!(streamed.is_ok(), "{streamed:?}");
        assert_eq!(streamed, scanned);
    }
    for malformed in [
        &b"<a><b></a>"[..],
        b"<a><b>",
        b"<!DOCTYPE a><a/>",
        b"<bagObject><Woonplaats><naam>&unknown;</naam></Woonplaats></bagObject>",
        b"<a b=\"1></a>",
        b"<a><!-- unterminated </a>",
    ] {
        let [streamed, scanned] = both_parsers(malformed);
        assert!(streamed.is_err(), "{}", malformed.escape_ascii());
        assert!(scanned.is_err(), "{}", malformed.escape_ascii());
    }
}

#[test]
fn skipping_ended_records_keeps_current_records_and_counts() {
    let data = fs::read(fixture()).unwrap();
    for date in [
        "2000-06-01",
        "2001-01-01",
        "2026-10-06",
        "2026-10-07",
        "2026-10-08",
    ] {
        let current = |skip: Option<&str>| {
            let mut records = Vec::new();
            let summary = xml::parse_bytes(&data, skip, |record| {
                if record.validity.is_current(date)? {
                    records.push(format!("{:?}", record.into_object(true)?));
                }
                Ok(())
            })
            .unwrap();
            (summary.records, records)
        };
        assert_eq!(current(Some(date)), current(None), "{date}");
    }
}
