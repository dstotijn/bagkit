use anyhow::{Context, Result, ensure};

/// A ring of RD New positions; the last position repeats the first.
pub type Ring = Vec<[f64; 2]>;

/// The EPSG code of RD New, the coordinate system of BAG geometry.
pub const RD_NEW: i32 = 28992;

/// Parses whitespace-separated coordinates with the given stride, keeping x and y.
pub fn parse_positions(text: &str, dimension: usize) -> Result<Ring> {
    ensure!(
        dimension == 2 || dimension == 3,
        "unsupported coordinate dimension {dimension}"
    );
    let mut positions = Vec::new();
    let mut position = [0.0; 3];
    let mut index = 0;
    for value in text.split_ascii_whitespace() {
        let value: f64 = value.parse().context("invalid coordinate")?;
        ensure!(value.is_finite(), "non-finite coordinate");
        position[index] = value;
        index += 1;
        if index == dimension {
            positions.push([position[0], position[1]]);
            index = 0;
        }
    }
    ensure!(
        index == 0 && !positions.is_empty(),
        "coordinates do not form {dimension}-dimensional positions"
    );
    Ok(positions)
}

/// The area-weighted centroid of a polygon whose interior rings are holes. Degenerate
/// polygons without area fall back to the mean of their distinct exterior vertices.
pub fn centroid(rings: &[Ring]) -> [f64; 2] {
    let exterior = &rings[0];
    // Work relative to the first vertex to keep products of RD coordinates small.
    let origin = exterior[0];
    let (mut area, mut x, mut y) = (0.0, 0.0, 0.0);
    for (index, ring) in rings.iter().enumerate() {
        let (mut ring_area, mut ring_x, mut ring_y) = (0.0, 0.0, 0.0);
        for pair in ring.windows(2) {
            let (x0, y0) = (pair[0][0] - origin[0], pair[0][1] - origin[1]);
            let (x1, y1) = (pair[1][0] - origin[0], pair[1][1] - origin[1]);
            let cross = x0 * y1 - x1 * y0;
            ring_area += cross;
            ring_x += (x0 + x1) * cross;
            ring_y += (y0 + y1) * cross;
        }
        // Orientation varies, so take the exterior as positive and holes as negative.
        let sign = if (index == 0) == (ring_area >= 0.0) {
            1.0
        } else {
            -1.0
        };
        area += sign * ring_area / 2.0;
        x += sign * ring_x / 6.0;
        y += sign * ring_y / 6.0;
    }
    if area.abs() > 1e-9 {
        return [origin[0] + x / area, origin[1] + y / area];
    }
    let distinct = if exterior.len() > 1 && exterior.first() == exterior.last() {
        &exterior[..exterior.len() - 1]
    } else {
        &exterior[..]
    };
    let count = distinct.len() as f64;
    [
        distinct.iter().map(|p| p[0]).sum::<f64>() / count,
        distinct.iter().map(|p| p[1]).sum::<f64>() / count,
    ]
}

/// Encodes a polygon as a GeoPackage geometry blob: the GeoPackage header with an XY
/// envelope, followed by little-endian WKB.
pub fn gpkg_polygon(rings: &[Ring], srs_id: i32) -> Vec<u8> {
    let positions: usize = rings.iter().map(Vec::len).sum();
    let mut blob = Vec::with_capacity(8 + 32 + 9 + 4 * rings.len() + 16 * positions);
    blob.extend_from_slice(b"GP");
    blob.push(0); // version 1
    blob.push(0b0000_0011); // little endian, envelope [minx, maxx, miny, maxy]
    blob.extend_from_slice(&srs_id.to_le_bytes());
    let [min_x, min_y, max_x, max_y] = envelope(&rings[0]);
    for value in [min_x, max_x, min_y, max_y] {
        blob.extend_from_slice(&value.to_le_bytes());
    }
    push_wkb(&mut blob, rings);
    blob
}

/// Encodes a polygon as little-endian WKB.
pub fn wkb_polygon(rings: &[Ring]) -> Vec<u8> {
    let positions: usize = rings.iter().map(Vec::len).sum();
    let mut blob = Vec::with_capacity(9 + 4 * rings.len() + 16 * positions);
    push_wkb(&mut blob, rings);
    blob
}

fn push_wkb(blob: &mut Vec<u8>, rings: &[Ring]) {
    blob.push(1); // little endian
    blob.extend_from_slice(&3u32.to_le_bytes()); // Polygon
    blob.extend_from_slice(&(rings.len() as u32).to_le_bytes());
    for ring in rings {
        blob.extend_from_slice(&(ring.len() as u32).to_le_bytes());
        for [x, y] in ring {
            blob.extend_from_slice(&x.to_le_bytes());
            blob.extend_from_slice(&y.to_le_bytes());
        }
    }
}

/// Writes a polygon as WKT, with coordinates as the BAG states them.
pub fn wkt_polygon(rings: &[Ring]) -> String {
    use std::fmt::Write;
    let mut text = String::from("POLYGON (");
    for (index, ring) in rings.iter().enumerate() {
        text.push_str(if index == 0 { "(" } else { ", (" });
        for (position, [x, y]) in ring.iter().enumerate() {
            if position > 0 {
                text.push_str(", ");
            }
            let _ = write!(text, "{x} {y}");
        }
        text.push(')');
    }
    text.push(')');
    text
}

/// `[min_x, min_y, max_x, max_y]` of a ring.
pub fn envelope(ring: &[[f64; 2]]) -> [f64; 4] {
    let mut envelope = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for [x, y] in ring {
        envelope[0] = envelope[0].min(*x);
        envelope[1] = envelope[1].min(*y);
        envelope[2] = envelope[2].max(*x);
        envelope[3] = envelope[3].max(*y);
    }
    envelope
}
