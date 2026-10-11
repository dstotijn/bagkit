//! Conversion between RD New and WGS84, as the EPSG Geodetic Parameter Dataset defines it.
//!
//! RD New (EPSG:28992) is an Oblique Stereographic projection (EPSG method 9809) of Amersfoort
//! (EPSG:4289) on the Bessel 1841 ellipsoid. Amersfoort to WGS 84 (4) (EPSG:4833) shifts it to
//! WGS84 with a Coordinate Frame rotation (EPSG method 9607), stated accurate to 1 m. The
//! formulas are those of IOGP Guidance Note 7-2, "Coordinate Conversions and Transformations
//! including Formulas". This is not the official, centimetre-accurate RDNAPTRANS™ procedure,
//! which needs correction grids; it is accurate enough to place and find adressen.

use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};
use std::sync::LazyLock;

/// The projection's derived constants, computed once.
static RD_NEW: LazyLock<Stereographic> = LazyLock::new(Stereographic::new);

/// An ellipsoid by its semi-major axis and inverse flattening.
struct Ellipsoid {
    a: f64,
    inverse_flattening: f64,
}

impl Ellipsoid {
    /// The first eccentricity squared.
    fn e2(&self) -> f64 {
        let f = 1.0 / self.inverse_flattening;
        f * (2.0 - f)
    }
}

/// Bessel 1841 (EPSG:7004).
const BESSEL: Ellipsoid = Ellipsoid {
    a: 6_377_397.155,
    inverse_flattening: 299.152_812_8,
};

/// WGS 84 (EPSG:7030).
const WGS84: Ellipsoid = Ellipsoid {
    a: 6_378_137.0,
    inverse_flattening: 298.257_223_563,
};

/// RD New's projection parameters (EPSG:28992), with angles in degrees.
const LATITUDE_OF_ORIGIN: f64 = 52.156_160_555_555_6;
const LONGITUDE_OF_ORIGIN: f64 = 5.387_638_888_888_89;
const SCALE: f64 = 0.999_907_9;
const FALSE_EASTING: f64 = 155_000.0;
const FALSE_NORTHING: f64 = 463_000.0;

/// Amersfoort to WGS 84 (4) (EPSG:4833): translations in metres, rotations in microradians and
/// the scale difference in parts per million, in the Coordinate Frame convention.
const TRANSLATION: [f64; 3] = [565.4171, 50.3319, 465.5524];
const ROTATION: [f64; 3] = [1.9342, -1.6677, 9.1019];
const SCALE_DIFFERENCE: f64 = 4.0725;

/// Converts RD New coordinates to WGS84 latitude and longitude in degrees.
pub fn rd_to_wgs84(x: f64, y: f64) -> (f64, f64) {
    let (lat, lon) = RD_NEW.reverse(x, y);
    let geocentric = to_geocentric(&BESSEL, lat, lon, 0.0);
    let shifted = helmert(geocentric);
    let (lat, lon, _) = from_geocentric(&WGS84, shifted);
    (lat.to_degrees(), lon.to_degrees())
}

/// Converts WGS84 latitude and longitude in degrees to RD New. Returns `None` outside the area
/// around the Netherlands where the transformation applies.
///
/// [`rd_to_wgs84`] starts on the Bessel ellipsoid's surface, which lies some tens of metres off
/// WGS84's. To invert it exactly, this finds the WGS84 height at which the point lands on the
/// Bessel surface too, so converting there and back returns the same coordinates.
pub fn wgs84_to_rd(lat: f64, lon: f64) -> Option<(f64, f64)> {
    if !(50.0..=54.0).contains(&lat) || !(3.0..=8.0).contains(&lon) {
        return None;
    }
    let (phi, lambda) = (lat.to_radians(), lon.to_radians());
    let mut height = 0.0;
    let mut bessel = (0.0, 0.0);
    for _ in 0..5 {
        let shifted = inverse_helmert(to_geocentric(&WGS84, phi, lambda, height));
        let (b_phi, b_lambda, b_height) = from_geocentric(&BESSEL, shifted);
        bessel = (b_phi, b_lambda);
        // A metre up on one ellipsoid is a metre up on the other, to well within a millimetre.
        height -= b_height;
        if b_height.abs() < 1e-6 {
            break;
        }
    }
    Some(RD_NEW.forward(bessel.0, bessel.1))
}

/// The Oblique Stereographic projection's constants for RD New (IOGP 7-2, method 9809).
struct Stereographic {
    e: f64,
    n: f64,
    c: f64,
    r: f64,
    chi0: f64,
    lambda0: f64,
}

impl Stereographic {
    fn new() -> Self {
        let e2 = BESSEL.e2();
        let e = e2.sqrt();
        let phi0 = LATITUDE_OF_ORIGIN.to_radians();
        let sin0 = phi0.sin();
        let rho0 = BESSEL.a * (1.0 - e2) / (1.0 - e2 * sin0 * sin0).powf(1.5);
        let nu0 = BESSEL.a / (1.0 - e2 * sin0 * sin0).sqrt();
        let r = (rho0 * nu0).sqrt();
        let n = (1.0 + e2 * phi0.cos().powi(4) / (1.0 - e2)).sqrt();
        let s1 = (1.0 + sin0) / (1.0 - sin0);
        let s2 = (1.0 - e * sin0) / (1.0 + e * sin0);
        let w1 = (s1 * s2.powf(e)).powf(n);
        let sin_chi00 = (w1 - 1.0) / (w1 + 1.0);
        let c = (n + sin0) * (1.0 - sin_chi00) / ((n - sin0) * (1.0 + sin_chi00));
        let w2 = c * w1;
        let chi0 = ((w2 - 1.0) / (w2 + 1.0)).asin();
        Stereographic {
            e,
            n,
            c,
            r,
            chi0,
            lambda0: LONGITUDE_OF_ORIGIN.to_radians(),
        }
    }

    /// Projects a Bessel latitude and longitude in radians to RD New.
    fn forward(&self, phi: f64, lambda: f64) -> (f64, f64) {
        let big_lambda = self.n * (lambda - self.lambda0) + self.lambda0;
        let sin = phi.sin();
        let sa = (1.0 + sin) / (1.0 - sin);
        let sb = (1.0 - self.e * sin) / (1.0 + self.e * sin);
        let w = self.c * (sa * sb.powf(self.e)).powf(self.n);
        let chi = ((w - 1.0) / (w + 1.0)).asin();
        let d = big_lambda - self.lambda0;
        let b = 1.0 + chi.sin() * self.chi0.sin() + chi.cos() * self.chi0.cos() * d.cos();
        let k = 2.0 * self.r * SCALE;
        (
            FALSE_EASTING + k * chi.cos() * d.sin() / b,
            FALSE_NORTHING
                + k * (chi.sin() * self.chi0.cos() - chi.cos() * self.chi0.sin() * d.cos()) / b,
        )
    }

    /// Unprojects RD New to a Bessel latitude and longitude in radians.
    fn reverse(&self, x: f64, y: f64) -> (f64, f64) {
        let k = 2.0 * self.r * SCALE;
        let (de, dn) = (x - FALSE_EASTING, y - FALSE_NORTHING);
        let g = k * (FRAC_PI_4 - self.chi0 / 2.0).tan();
        let h = 2.0 * k * self.chi0.tan() + g;
        let i = (de / (h + dn)).atan();
        let j = (de / (g - dn)).atan() - i;
        let chi = self.chi0 + 2.0 * ((dn - de * (j / 2.0).tan()) / k).atan();
        let big_lambda = j + 2.0 * i + self.lambda0;
        let lambda = (big_lambda - self.lambda0) / self.n + self.lambda0;

        // The conformal latitude back to the geodetic latitude, iterating until it settles.
        let sin_chi = chi.sin();
        let psi = 0.5 * ((1.0 + sin_chi) / (self.c * (1.0 - sin_chi))).ln() / self.n;
        let e2 = self.e * self.e;
        let mut phi = 2.0 * psi.exp().atan() - FRAC_PI_2;
        for _ in 0..10 {
            let sin = phi.sin();
            let psi_i = ((phi / 2.0 + FRAC_PI_4).tan()
                * ((1.0 - self.e * sin) / (1.0 + self.e * sin)).powf(self.e / 2.0))
            .ln();
            let next = phi - (psi_i - psi) * phi.cos() * (1.0 - e2 * sin * sin) / (1.0 - e2);
            let settled = (next - phi).abs() < 1e-14;
            phi = next;
            if settled {
                break;
            }
        }
        (phi, lambda)
    }
}

/// Geographic coordinates in radians, and a height in metres above the ellipsoid, to geocentric
/// (IOGP 7-2, method 9602).
fn to_geocentric(ellipsoid: &Ellipsoid, phi: f64, lambda: f64, height: f64) -> [f64; 3] {
    let e2 = ellipsoid.e2();
    let nu = ellipsoid.a / (1.0 - e2 * phi.sin().powi(2)).sqrt();
    [
        (nu + height) * phi.cos() * lambda.cos(),
        (nu + height) * phi.cos() * lambda.sin(),
        ((1.0 - e2) * nu + height) * phi.sin(),
    ]
}

/// Geocentric coordinates to geographic latitude and longitude in radians and the height above
/// the ellipsoid, with Bowring's formula as IOGP 7-2 gives it for method 9602; near the Earth's
/// surface it is exact to well under a millimetre without iterating.
fn from_geocentric(ellipsoid: &Ellipsoid, [x, y, z]: [f64; 3]) -> (f64, f64, f64) {
    let e2 = ellipsoid.e2();
    let a = ellipsoid.a;
    let b = a * (1.0 - 1.0 / ellipsoid.inverse_flattening);
    let epsilon = e2 / (1.0 - e2);
    let p = x.hypot(y);
    let q = (z * a).atan2(p * b);
    let phi = (z + epsilon * b * q.sin().powi(3)).atan2(p - e2 * a * q.cos().powi(3));
    let nu = a / (1.0 - e2 * phi.sin().powi(2)).sqrt();
    (phi, y.atan2(x), p / phi.cos() - nu)
}

/// The Coordinate Frame rotation's matrix, scaled: rotations in radians and 1 plus the scale
/// difference (IOGP 7-2, method 9607).
fn helmert_matrix() -> [[f64; 3]; 3] {
    let [rx, ry, rz] = ROTATION.map(|r| r * 1e-6);
    let m = 1.0 + SCALE_DIFFERENCE * 1e-6;
    [
        [m, m * rz, -m * ry],
        [-m * rz, m, m * rx],
        [m * ry, -m * rx, m],
    ]
}

/// Shifts Amersfoort geocentric coordinates to WGS84.
fn helmert(point: [f64; 3]) -> [f64; 3] {
    let matrix = helmert_matrix();
    std::array::from_fn(|row| {
        TRANSLATION[row] + (0..3).map(|col| matrix[row][col] * point[col]).sum::<f64>()
    })
}

/// Shifts WGS84 geocentric coordinates to Amersfoort, solving the forward shift exactly rather
/// than reversing the parameters' signs.
fn inverse_helmert(point: [f64; 3]) -> [f64; 3] {
    let [[a, b, c], [d, e, f], [g, h, i]] = helmert_matrix();
    let [u, v, w] = std::array::from_fn(|row| point[row] - TRANSLATION[row]);
    let determinant = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    [
        (u * (e * i - f * h) - b * (v * i - f * w) + c * (v * h - e * w)) / determinant,
        (a * (v * i - f * w) - u * (d * i - f * g) + c * (d * w - v * g)) / determinant,
        (a * (e * w - v * h) - b * (d * w - v * g) + u * (d * h - e * g)) / determinant,
    ]
}
