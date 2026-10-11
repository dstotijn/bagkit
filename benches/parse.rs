//! Parses XML documents held in memory and reports throughput, one thread at a time.
use anyhow::{Context, Result, ensure};
use bagkit::xml;
use std::time::Instant;

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().filter(|a| a != "--bench").collect();
    ensure!(
        args.len() > 1,
        "usage: cargo bench --bench parse -- XML... [--rounds N]"
    );
    let mut rounds = 3;
    let mut documents = Vec::new();
    let mut paths = args[1..].iter();
    while let Some(path) = paths.next() {
        if path == "--rounds" {
            rounds = paths.next().context("--rounds needs a number")?.parse()?;
        } else {
            documents.push(std::fs::read(path).with_context(|| path.clone())?);
        }
    }
    let bytes: usize = documents.iter().map(Vec::len).sum();
    for round in 1..=rounds {
        let started = Instant::now();
        let mut records = 0;
        for document in &documents {
            records += xml::parse_bytes(document, None, |record| {
                std::hint::black_box(record);
                Ok(())
            })?
            .records;
        }
        let seconds = started.elapsed().as_secs_f64();
        println!(
            "round {round}: {records} records, {:.1} MiB in {seconds:.3}s, {:.0} MiB/s",
            bytes as f64 / 1048576.0,
            bytes as f64 / 1048576.0 / seconds
        );
    }
    Ok(())
}
