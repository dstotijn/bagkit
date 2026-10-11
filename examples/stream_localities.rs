//! Prints the naam of every current woonplaats.
//!
//! cargo run --example stream_localities -- lvbag-extract-nl.zip 2026-10-08
use bagkit::{
    extract::{Extract, ObjectType, Selection},
    xml::BagObject,
};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let input = args.next().unwrap_or("lvbag-extract-nl.zip".into());
    let date = args.next().unwrap_or("2026-10-08".into());

    let extract = Extract::new(vec![input.into()], &date)?;
    let mut localities = Vec::new();
    extract.for_each(Selection::only(&[ObjectType::Locality]), |object| {
        if let BagObject::Locality(locality) = object {
            localities.push(locality.name);
        }
        Ok(())
    })?;
    localities.sort();
    println!(
        "{} woonplaatsen, from {:?}",
        localities.len(),
        localities.first()
    );
    Ok(())
}
