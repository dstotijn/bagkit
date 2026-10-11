//! Keeps the OpenAPI description and the code in step: the description's paths and
//! parameters are the ones `api::route` accepts, each operation routes with the
//! description's example values, and its example response has the shape of a real one.
//! `tests/http.rs` checks live responses against the description's schemas.
mod common;

use bagkit::{api, query};
use common::{ImportOptions, assert_matches_schema, import};
use serde_json::Value;
use std::{collections::BTreeSet, path::PathBuf};

fn description() -> Value {
    serde_json::from_str(api::OPENAPI).unwrap()
}

/// A value with its `$ref` followed, if it has one.
fn resolve<'a>(spec: &'a Value, value: &'a Value) -> &'a Value {
    match value["$ref"].as_str() {
        Some(reference) => spec.pointer(reference.strip_prefix('#').unwrap()).unwrap(),
        None => value,
    }
}

/// The `GET` parameters of every documented path.
fn parameters(spec: &Value) -> Vec<(String, Vec<&Value>)> {
    spec["paths"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(path, item)| {
            let parameters = item["get"]["parameters"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| resolve(spec, p))
                .collect();
            (path.clone(), parameters)
        })
        .collect()
}

/// A parameter's example as it appears in a request: its `example`, or else its schema's
/// default.
fn example(spec: &Value, parameter: &Value) -> String {
    let value = if parameter["example"].is_null() {
        &resolve(spec, &parameter["schema"])["default"]
    } else {
        &parameter["example"]
    };
    match value {
        Value::String(text) => text.clone(),
        Value::Null => panic!("{} has no example or default", parameter["name"]),
        other => other.to_string(),
    }
}

#[test]
fn documented_paths_and_query_parameters_are_the_ones_the_code_accepts() {
    let spec = description();
    let documented: BTreeSet<(String, BTreeSet<(String, bool)>)> = parameters(&spec)
        .into_iter()
        .map(|(path, parameters)| {
            let query = parameters
                .into_iter()
                .filter(|p| p["in"] == "query")
                .map(|p| {
                    (
                        p["name"].as_str().unwrap().to_owned(),
                        p["required"] == true,
                    )
                })
                .collect();
            (path, query)
        })
        .collect();
    let accepted: BTreeSet<(String, BTreeSet<(String, bool)>)> = api::RESOURCES
        .iter()
        .map(|(path, parameters)| {
            let query = parameters
                .iter()
                .map(|(name, required)| ((*name).to_owned(), *required))
                .collect();
            ((*path).to_owned(), query)
        })
        .collect();
    assert_eq!(documented, accepted);
}

#[test]
fn every_documented_operation_routes_with_its_example_values() {
    let spec = description();
    for (path, parameters) in parameters(&spec) {
        let mut target = format!("{}{path}", api::BASE_PATH);
        let mut query = Vec::new();
        for parameter in parameters {
            let name = parameter["name"].as_str().unwrap();
            let value = example(&spec, parameter);
            match parameter["in"].as_str() {
                Some("path") => target = target.replace(&format!("{{{name}}}"), &value),
                Some("query") => query.push(format!("{name}={value}")),
                other => panic!("{path}: {name} is in {other:?}"),
            }
        }
        if !query.is_empty() {
            target = format!("{target}?{}", query.join("&"));
        }
        if let Err(response) = api::route("GET", &target, None) {
            panic!("{target}: {}", response.body);
        }
    }
}

#[test]
fn the_descriptions_example_response_has_the_shape_of_a_real_one() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("bag.sqlite");
    import(
        &[PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bag.xml")],
        &ImportOptions {
            output: db.clone(),
            date: "2026-10-07".into(),
            force: false,
            cache_mib: 8,
            geometry: true,
            threads: 0,
            address_list: None,
        },
    )
    .unwrap();
    let response = api::get(
        &query::open(&db).unwrap(),
        "/v1/adressen?postcode=7311KZ&huisnummer=110&huisletter=A&huisnummertoevoeging=2",
    )
    .unwrap();
    let real: Value = serde_json::from_str(&response.body).unwrap();
    let spec = description();
    let documented = &spec["components"]["schemas"]["AdresCollectie"]["example"];
    assert_matches_schema("AdresCollectie", documented);
    assert_eq!(common::shape(documented), common::shape(&real));
}
