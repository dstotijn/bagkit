//! Data bagkit derives from BAG voorkomens.
use crate::{geo::rd_to_wgs84, geometry::centroid, xml::Geometry};

/// An adresseerbaar object's punt in RD New: the punt the BAG records, or its vlak's
/// area-weighted centroid. Also its WGS84 breedtegraad and lengtegraad when the punt lies
/// inside RD New's area of use; some ingetrokken and niet gerealiseerde verblijfsobjecten
/// carry placeholder coordinates such as 0 0, which keep their punt but get no WGS84 punt.
pub fn object_point(geometry: Option<&Geometry>) -> (Option<[f64; 2]>, Option<[f64; 2]>) {
    let point = match geometry {
        Some(Geometry::Point(point)) => Some(*point),
        Some(Geometry::Polygon(rings)) => Some(centroid(rings)),
        None => None,
    };
    let wgs84 = point
        .filter(|[x, y]| (-7000.0..=300_000.0).contains(x) && (289_000.0..=629_000.0).contains(y))
        .map(|[x, y]| {
            let (lat, lon) = rd_to_wgs84(x, y);
            [lat, lon]
        });
    (point, wgs84)
}
