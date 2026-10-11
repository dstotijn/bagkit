use bagkit::rtree;
use rusqlite::Connection;

/// Builds a bulk-loaded tree over `count` pseudo-random points and checks it with SQLite.
fn check(count: i64) {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE VIRTUAL TABLE t USING rtree_i32 (id, min_x, max_x, min_y, max_y);
         CREATE TABLE points (id INTEGER PRIMARY KEY, x INTEGER, y INTEGER);",
    )
    .unwrap();
    let mut seed = 42u64;
    let mut next = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as i64
    };
    let mut entries = Vec::new();
    for id in 0..count {
        let (x, y) = (next() % 300_000_000, 289_000_000 + next() % 340_000_000);
        db.execute("INSERT INTO points VALUES (?1, ?2, ?3)", [id * 7 + 3, x, y])
            .unwrap();
        entries.push((id * 7 + 3, x as i32, y as i32));
    }
    rtree::bulk_load(&db, "t", entries).unwrap();
    let report: String = db
        .query_row("SELECT rtreecheck('t')", [], |r| r.get(0))
        .unwrap();
    assert_eq!(report, "ok", "{count} points");
    let indexed: i64 = db
        .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
        .unwrap();
    assert_eq!(indexed, count);
    for window in [
        [0, 300_000_000, 289_000_000, 629_000_000],
        [100_000_000, 120_000_000, 400_000_000, 430_000_000],
        [5_000_000, 5_500_000, 300_000_000, 300_500_000],
    ] {
        let from_tree: Vec<i64> = db
            .prepare(
                "SELECT id FROM t
                 WHERE min_x >= ?1 AND max_x <= ?2 AND min_y >= ?3 AND max_y <= ?4
                 ORDER BY id",
            )
            .unwrap()
            .query_map(window, |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let scanned: Vec<i64> = db
            .prepare(
                "SELECT id FROM points
                 WHERE x >= ?1 AND x <= ?2 AND y >= ?3 AND y <= ?4
                 ORDER BY id",
            )
            .unwrap()
            .query_map(window, |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(from_tree, scanned, "{count} points, window {window:?}");
    }
    // The tree stays writable after bulk loading.
    db.execute("INSERT INTO t VALUES (-1, 5, 5, 6, 6)", [])
        .unwrap();
    let report: String = db
        .query_row("SELECT rtreecheck('t')", [], |r| r.get(0))
        .unwrap();
    assert_eq!(report, "ok");
}

#[test]
fn bulk_loaded_trees_pass_sqlite_checks_at_every_size() {
    for count in [
        0,
        1,
        50,
        51,
        52,
        51 * 51,
        51 * 51 + 1,
        51 * 51 * 2 + 7,
        20_000,
    ] {
        check(count);
    }
}
