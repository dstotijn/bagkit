//! Gemeente names and provincies, and each woonplaats's gemeente.
//!
//! The BAG names no gemeenten. Kadaster supplies each woonplaats's gemeentecode in
//! the extract's GEM-WPL-RELATIE; the names come from a table built into bagkit
//! (`src/municipalities.tsv`, generated from PDOK by `scripts/municipalities.py`).
use crate::xml::{LocalityMunicipality, bag_id};
use anyhow::{Result, bail};
use std::collections::{BTreeSet, HashMap};

const TABLE: &str = include_str!("municipalities.tsv");

/// Where the built-in table came from, from its header line.
pub const SOURCE: &str = {
    let bytes = TABLE.as_bytes();
    let mut end = 0;
    while bytes[end] != b'\n' {
        end += 1;
    }
    match TABLE.split_at(end).0.split_at_checked(2) {
        Some((_, source)) => source,
        None => "",
    }
};

/// A gemeente from the built-in table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Municipality {
    /// The gemeentecode.
    pub id: i64,
    /// The naam.
    pub name: &'static str,
    /// The provincie's naam.
    pub province: &'static str,
}

/// Every gemeente in the built-in table.
pub fn all() -> impl Iterator<Item = Municipality> {
    TABLE.lines().skip(2).map(|line| {
        let mut fields = line.split('\t');
        let mut next = || fields.next().expect("municipalities.tsv has three columns");
        Municipality {
            id: next().parse().expect("municipality codes are numbers"),
            name: next(),
            province: next(),
        }
    })
}

/// Each current woonplaats's gemeentecode. Without relations every woonplaats has none;
/// with them, every woonplaats needs exactly one. Relations for woonplaatsen that are not
/// current are ignored. Codes missing from the built-in table are reported, since they
/// will have no name.
pub fn assign(
    relations: &[LocalityMunicipality],
    localities: impl IntoIterator<Item = i64>,
) -> Result<HashMap<i64, i64>> {
    let mut by_locality = HashMap::with_capacity(relations.len());
    for relation in relations {
        if by_locality
            .insert(relation.locality_id, relation.municipality_id)
            .is_some()
        {
            bail!(
                "Woonplaats {} belongs to more than one gemeente in GEM-WPL-RELATIE",
                bag_id(relation.locality_id, 4)
            );
        }
    }
    let mut assigned = HashMap::new();
    let mut without = BTreeSet::new();
    for locality in localities {
        match by_locality.get(&locality) {
            Some(&municipality) => {
                assigned.insert(locality, municipality);
            }
            None => {
                without.insert(locality);
            }
        }
    }
    if relations.is_empty() {
        return Ok(HashMap::new());
    }
    if let Some(first) = without.first() {
        bail!(
            "{} woonplaatsen have no gemeente in GEM-WPL-RELATIE, for example {}",
            without.len(),
            bag_id(*first, 4)
        );
    }
    let known: BTreeSet<i64> = all().map(|m| m.id).collect();
    let unnamed: BTreeSet<_> = assigned.values().filter(|m| !known.contains(m)).collect();
    if !unnamed.is_empty() {
        eprintln!(
            "{} municipalities are missing from bagkit's table and have no name or province; \
             regenerate it with scripts/municipalities.py",
            unnamed.len()
        );
    }
    Ok(assigned)
}
