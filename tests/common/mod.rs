//! Runs conversions the way the tests describe them.
#![allow(dead_code)]
use bagkit::{
    convert::{self, Options, Shape},
    dataset::Format,
    extract::{Extract, Selection},
};
use std::path::PathBuf;

/// A conversion: SQLite tables, unless `address_list` names a format for an address list.
pub struct ImportOptions {
    pub output: PathBuf,
    pub date: String,
    pub force: bool,
    pub cache_mib: u32,
    pub geometry: bool,
    pub threads: usize,
    pub address_list: Option<Format>,
}

#[derive(Debug)]
pub struct ImportStats {
    pub records: u64,
    pub current: u64,
    pub points_outside_rd_new: u64,
    pub rows: u64,
}

pub fn import(paths: &[PathBuf], options: &ImportOptions) -> anyhow::Result<ImportStats> {
    let mut extract = Extract::new(paths.to_vec(), &options.date)?;
    extract.threads = options.threads;
    extract.footprints = options.geometry;
    let (format, shape) = match options.address_list {
        Some(format) => (format, Shape::Addresses),
        None => (Format::Sqlite, Shape::Tables),
    };
    let stats = convert::convert(
        &extract,
        &Options {
            output: options.output.clone(),
            force: options.force,
            format,
            shape,
            selection: Selection::all(),
            cache_mib: options.cache_mib,
        },
    )?;
    Ok(ImportStats {
        records: stats.read.records,
        current: stats.read.current,
        points_outside_rd_new: stats.points_outside_rd_new,
        rows: stats.rows.unwrap_or_default(),
    })
}

/// The keys of every object in a JSON value, by path: its shape, without its values.
pub fn shape(value: &serde_json::Value) -> std::collections::BTreeSet<String> {
    fn walk(value: &serde_json::Value, path: &str, keys: &mut std::collections::BTreeSet<String>) {
        match value {
            serde_json::Value::Object(object) => {
                for (key, value) in object {
                    let path = format!("{path}.{key}");
                    keys.insert(path.clone());
                    walk(value, &path, keys);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    walk(item, &format!("{path}[]"), keys);
                }
            }
            _ => (),
        }
    }
    let mut keys = std::collections::BTreeSet::new();
    walk(value, "", &mut keys);
    keys
}

/// Checks a JSON value against a schema in the OpenAPI description's components, and
/// panics with every mismatch. Handles the subset of OpenAPI 3.0 schemas the description
/// uses; properties a schema does not list count as mismatches, so undocumented fields
/// fail too.
pub fn assert_matches_schema(name: &str, value: &serde_json::Value) {
    let spec: serde_json::Value = serde_json::from_str(bagkit::api::OPENAPI).unwrap();
    let schema = serde_json::json!({ "$ref": format!("#/components/schemas/{name}") });
    let mut errors = Vec::new();
    check_schema(&spec, &schema, value, "$", &mut errors);
    assert!(errors.is_empty(), "{name}: {errors:#?}\n{value:#}");
}

fn check_schema(
    spec: &serde_json::Value,
    schema: &serde_json::Value,
    value: &serde_json::Value,
    path: &str,
    errors: &mut Vec<String>,
) {
    use serde_json::Value;
    if let Some(reference) = schema["$ref"].as_str() {
        let pointer = reference.strip_prefix('#').unwrap();
        let target = spec
            .pointer(pointer)
            .unwrap_or_else(|| panic!("{reference}"));
        return check_schema(spec, target, value, path, errors);
    }
    if value.is_null() {
        if schema["nullable"] != true {
            errors.push(format!("{path} is null"));
        }
        return;
    }
    for part in schema["allOf"].as_array().into_iter().flatten() {
        check_schema(spec, part, value, path, errors);
    }
    if let Some(options) = schema["enum"].as_array()
        && !options.contains(value)
    {
        errors.push(format!("{path} is {value}, not one of {options:?}"));
    }
    if let (Some(pattern), Some(text)) = (schema["pattern"].as_str(), value.as_str())
        && !regex::Regex::new(pattern).unwrap().is_match(text)
    {
        errors.push(format!("{path} is {value}, which does not match {pattern}"));
    }
    let matches = match schema["type"].as_str() {
        None => true,
        Some("string") => value.is_string(),
        Some("integer") => value.is_i64() || value.is_u64(),
        Some("number") => value.is_number(),
        Some("boolean") => value.is_boolean(),
        Some("array") => value.is_array(),
        Some("object") => value.is_object(),
        Some(other) => panic!("unsupported type {other}"),
    };
    if !matches {
        errors.push(format!("{path} is {value}, not {}", schema["type"]));
        return;
    }
    match value {
        Value::Object(object) => {
            for required in schema["required"].as_array().into_iter().flatten() {
                if !object.contains_key(required.as_str().unwrap()) {
                    errors.push(format!("{path}.{required} is missing"));
                }
            }
            let properties = schema["properties"].as_object();
            // Schemas without properties, such as the OpenAPI description's own, allow any.
            if let Some(properties) = properties {
                for (key, item) in object {
                    match properties.get(key) {
                        Some(property) => {
                            check_schema(spec, property, item, &format!("{path}.{key}"), errors)
                        }
                        None if schema["allOf"].is_null() => {
                            errors.push(format!("{path}.{key} is not documented"))
                        }
                        None => (),
                    }
                }
            }
        }
        Value::Array(items) => {
            if let Some(min) = schema["minItems"].as_u64()
                && (items.len() as u64) < min
            {
                errors.push(format!("{path} has fewer than {min} items"));
            }
            if let Some(max) = schema["maxItems"].as_u64()
                && (items.len() as u64) > max
            {
                errors.push(format!("{path} has more than {max} items"));
            }
            for (index, item) in items.iter().enumerate() {
                check_schema(
                    spec,
                    &schema["items"],
                    item,
                    &format!("{path}[{index}]"),
                    errors,
                );
            }
        }
        _ => (),
    }
}
