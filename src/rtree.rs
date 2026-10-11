//! Bulk loading for SQLite `rtree_i32` tables with two dimensions.
//!
//! Inserting points one at a time makes SQLite's R*Tree choose subtrees and split nodes for
//! every entry. Instead, entries are sorted along a Hilbert curve and packed into full
//! nodes, which are written directly to the table's shadow tables in the format described
//! in SQLite's `rtree.c`: each node is a fixed-size blob with a two-byte depth (root only),
//! a two-byte cell count, and cells of a big-endian 64-bit id followed by four big-endian
//! 32-bit coordinates. `rtreecheck()` validates the result in tests.
use anyhow::{Result, ensure};
use rusqlite::{Connection, params};

/// Cells per node: SQLite caps R*Tree nodes at 51 cells.
const CELLS: usize = 51;
/// Bytes per cell: an id and four 32-bit coordinates.
const CELL_BYTES: usize = 8 + 4 * 4;
/// Node blob size SQLite derives for 51 cells; every node must have the root's size.
const NODE_BYTES: usize = 4 + CELL_BYTES * CELLS;

/// One cell: an entry or child node id, and its bounds `[min_x, max_x, min_y, max_y]`.
type Cell = (i64, [i32; 4]);

/// Orders RD New millimetre coordinates along a Hilbert curve over the Netherlands, with
/// a 16-bit grid (about 5 m cells).
pub fn hilbert_key(x: i32, y: i32) -> u64 {
    hilbert(
        grid(i64::from(x), -7_000_000, 300_000_000),
        grid(i64::from(y), 289_000_000, 629_000_000),
    )
}

fn grid(value: i64, min: i64, max: i64) -> u32 {
    ((value.clamp(min, max) - min) * 65535 / (max - min)) as u32
}

/// The distance of `(x, y)` along a Hilbert curve filling a 65536 by 65536 grid.
fn hilbert(mut x: u32, mut y: u32) -> u64 {
    const N: u32 = 1 << 16;
    let mut d = 0u64;
    let mut s = N / 2;
    while s > 0 {
        let rx = u32::from(x & s > 0);
        let ry = u32::from(y & s > 0);
        d += u64::from(s) * u64::from(s) * u64::from((3 * rx) ^ ry);
        if ry == 0 {
            if rx == 1 {
                x = N - 1 - x;
                y = N - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        s /= 2;
    }
    d
}

fn bounds(cells: &[Cell]) -> [i32; 4] {
    let mut b = [i32::MAX, i32::MIN, i32::MAX, i32::MIN];
    for (_, c) in cells {
        b[0] = b[0].min(c[0]);
        b[1] = b[1].max(c[1]);
        b[2] = b[2].min(c[2]);
        b[3] = b[3].max(c[3]);
    }
    b
}

fn node(depth: u16, cells: &[Cell]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(NODE_BYTES);
    blob.extend_from_slice(&depth.to_be_bytes());
    blob.extend_from_slice(&(cells.len() as u16).to_be_bytes());
    for (id, coordinates) in cells {
        blob.extend_from_slice(&id.to_be_bytes());
        for c in coordinates {
            blob.extend_from_slice(&c.to_be_bytes());
        }
    }
    blob.resize(NODE_BYTES, 0);
    blob
}

/// Fills the empty `rtree_i32` table `table` with point entries `(id, x, y)`. Sorting
/// happens in memory: about 24 bytes per entry.
pub fn bulk_load(db: &Connection, table: &str, mut entries: Vec<(i64, i32, i32)>) -> Result<()> {
    let count: i64 = db.query_row(
        &format!("SELECT COUNT(*) FROM \"{table}_rowid\""),
        [],
        |r| r.get(0),
    )?;
    ensure!(count == 0, "{table} is not empty");
    if entries.is_empty() {
        return Ok(()); // An empty tree keeps the empty root SQLite created.
    }
    entries.sort_unstable_by_key(|&(id, x, y)| (hilbert_key(x, y), id));
    let mut write_node = db.prepare(&format!(
        "INSERT OR REPLACE INTO \"{table}_node\" VALUES (?1, ?2)"
    ))?;
    let mut write_parent =
        db.prepare(&format!("INSERT INTO \"{table}_parent\" VALUES (?1, ?2)"))?;
    let mut write_rowid = db.prepare(&format!("INSERT INTO \"{table}_rowid\" VALUES (?1, ?2)"))?;
    // Node 1 is the root. A single leaf is the root itself; otherwise leaves are numbered
    // from 2 in Hilbert order, followed by the inner levels.
    let single_leaf = entries.len() <= CELLS;
    let mut next_node = 2i64;
    let mut level: Vec<Cell> = Vec::with_capacity(entries.len().div_ceil(CELLS));
    let mut cells = Vec::with_capacity(CELLS);
    for chunk in entries.chunks(CELLS) {
        cells.clear();
        cells.extend(chunk.iter().map(|&(id, x, y)| (id, [x, x, y, y])));
        let id = if single_leaf { 1 } else { next_node };
        next_node += 1;
        write_node.execute(params![id, node(0, &cells)])?;
        level.push((id, bounds(&cells)));
    }
    // Write each entry's leaf in id order, which appends to the rowid table.
    // Entries and leaves have the same size, so this reuses the entries' allocation.
    let mut leaves: Vec<(i64, i64)> = entries
        .into_iter()
        .enumerate()
        .map(|(position, (id, _, _))| {
            let leaf = if single_leaf {
                1
            } else {
                2 + (position / CELLS) as i64
            };
            (id, leaf)
        })
        .collect();
    leaves.sort_unstable();
    for (id, leaf) in leaves {
        write_rowid.execute(params![id, leaf])?;
    }
    let mut depth = 0u16;
    while level.len() > 1 {
        depth += 1;
        let root = level.len() <= CELLS;
        let mut parents = Vec::with_capacity(level.len().div_ceil(CELLS));
        for children in level.chunks(CELLS) {
            let id = if root { 1 } else { next_node };
            next_node += 1;
            write_node.execute(params![id, node(if root { depth } else { 0 }, children)])?;
            for (child, _) in children {
                write_parent.execute(params![child, id])?;
            }
            parents.push((id, bounds(children)));
        }
        level = parents;
    }
    Ok(())
}
