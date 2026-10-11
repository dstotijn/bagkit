//! Writes an adreslijst as CSV, like `bagkit convert --shape addresses --format csv`.
//!
//! cargo run --example write_address_list -- lvbag-extract-nl.zip 2026-10-08
use bagkit::{
    convert::{self, Options, Shape},
    dataset::Format,
    extract::{Extract, Selection},
};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let input = args.next().unwrap_or("lvbag-extract-nl.zip".into());
    let date = args.next().unwrap_or("2026-10-08".into());

    let extract = Extract::new(vec![input.into()], &date)?;
    let options = Options {
        output: "adreslijst.csv".into(),
        force: true,
        format: Format::Csv,
        shape: Shape::Addresses,
        selection: Selection::all(),
        cache_mib: 64,
    };
    let stats = convert::convert(&extract, &options)?;
    println!("{} rows", stats.rows.unwrap_or_default());
    Ok(())
}
