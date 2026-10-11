use anyhow::{Context, Result, ensure};
use bagkit::{api, query};
use serde_json::json;
use std::{path::Path, time::Instant};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().filter(|a| a != "--bench").collect();
    let source = Path::new(
        args.get(1)
            .context("usage: cargo bench --bench lookup -- DATABASE [ITERATIONS]")?,
    );
    let iterations: usize = args.get(2).map(|s| s.parse()).transpose()?.unwrap_or(100);
    ensure!(iterations > 0, "iterations must be positive");
    let db = query::open(source)?;
    let (lat, lon): (f64, f64) = db.query_row(
        "SELECT breedtegraad, lengtegraad
         FROM adresseerbare_objecten
         WHERE breedtegraad IS NOT NULL
         ORDER BY id
         LIMIT 1
         OFFSET (
             SELECT MIN(COUNT(*) - 1, COUNT(*) / 2 + 49)
             FROM adresseerbare_objecten
             WHERE breedtegraad IS NOT NULL
         )",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (lat, lon) = (lat + 0.000001, lon + 0.000001);
    let nearby = format!("/v1/adressen/_dichtstbij?punt={lon},{lat}");
    let (postcode, number): (String, i64) = db.query_row(
        "SELECT postcode, huisnummer
         FROM adressen
         WHERE postcode IS NOT NULL
             AND nummeraanduiding_status != 'Naamgeving ingetrokken'
         LIMIT 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let address = format!("/v1/adressen?postcode={postcode}&huisnummer={number}");
    for target in [&nearby, &address] {
        let response = api::get(&db, target)?;
        ensure!(
            response.status == 200 && !response.body.contains("\"adressen\":[]"),
            "{target} found nothing"
        );
    }
    println!(
        "{}",
        json!({"sqlite":rusqlite::version(),"database_bytes":std::fs::metadata(source)?.len(),"lat":lat,"lon":lon,"postcode":postcode,"huisnummer":number,"iterations":iterations,"note":"warm API requests: routing, lookup and JSON serialization"})
    );
    for round in 1..=3 {
        for (name, target) in [("coordinates", &nearby), ("address", &address)] {
            for _ in 0..5 {
                std::hint::black_box(api::get(&db, target)?);
            }
            let start = Instant::now();
            for _ in 0..iterations {
                std::hint::black_box(api::get(&db, target)?);
            }
            println!(
                "{}",
                json!({"round":round,"lookup":name,"elapsed_seconds":start.elapsed().as_secs_f64(),"microseconds_per_lookup":start.elapsed().as_secs_f64()*1e6/iterations as f64})
            );
        }
    }
    Ok(())
}
