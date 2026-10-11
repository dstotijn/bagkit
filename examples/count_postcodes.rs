//! Counts current nummeraanduidingen per postcode, converting voorkomens on worker threads.
//!
//! cargo run --example count_postcodes -- lvbag-extract-nl.zip 2026-10-08
use anyhow::Result;
use bagkit::{
    extract::{Converter, Extract, ObjectType, Part, Selection},
    xml::BagObject,
};
use std::collections::BTreeMap;

/// Emits the postcode of every current nummeraanduiding.
struct Postcodes;

impl Converter for Postcodes {
    type State = ();
    type Item = String;

    fn begin(&self) {}

    fn convert(
        &self,
        _: &mut (),
        object: BagObject,
        emit: &mut dyn FnMut(String) -> Result<()>,
    ) -> Result<()> {
        if let BagObject::Address(address) = object
            && let Some(postcode) = address.postcode
        {
            emit(postcode)?;
        }
        Ok(())
    }
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let input = args.next().unwrap_or("lvbag-extract-nl.zip".into());
    let date = args.next().unwrap_or("2026-10-08".into());

    let extract = Extract::new(vec![input.into()], &date)?;
    let mut counts = BTreeMap::new();
    // Read both parts, in order, to see every voorkomen.
    for part in [Part::First, Part::Second] {
        extract.read(
            part,
            Selection::only(&[ObjectType::Address]),
            &Postcodes,
            &mut |postcode| {
                *counts.entry(postcode).or_insert(0u64) += 1;
                Ok(())
            },
        )?;
    }
    let busiest = counts.iter().max_by_key(|(_, count)| **count);
    println!("{} postcodes; the busiest is {busiest:?}", counts.len());
    Ok(())
}
