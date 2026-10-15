//! EPSG:25832 (ETRS89 / UTM zone 32N) → WGS84, pure Rust.
//!
//! Krüger series (n-series, third order) on the GRS80 ellipsoid — the
//! textbook inverse transverse-Mercator. Third order is accurate to well
//! under a millimetre inside a UTM zone, and ETRS89 vs WGS84 differ by less
//! than a metre in Europe, so the result is exact for our purpose (a
//! building's position for delivery-zone checks and routing).

/// GRS80 semi-major axis (m).
const A: f64 = 6_378_137.0;
/// GRS80 inverse flattening.
const INV_F: f64 = 298.257_222_101;
/// UTM scale factor on the central meridian.
const K0: f64 = 0.9996;
/// UTM false easting (m).
const FALSE_EASTING: f64 = 500_000.0;

/// Convert UTM (zone 32 north, metres) to WGS84 `(lat, lon)` in degrees.
pub fn utm32n_to_wgs84(east: f64, north: f64) -> (f64, f64) {
    utm_to_wgs84(east, north, 32)
}

/// Convert UTM easting/northing in the given northern-hemisphere zone to
/// WGS84 `(lat, lon)` in degrees.
pub fn utm_to_wgs84(east: f64, north: f64, zone: u8) -> (f64, f64) {
    let f = 1.0 / INV_F;
    let n = f / (2.0 - f);
    let n2 = n * n;
    let n3 = n2 * n;
    let n4 = n3 * n;
    // Rectifying radius.
    let a_rect = A / (1.0 + n) * (1.0 + n2 / 4.0 + n4 / 64.0);

    let xi = north / (K0 * a_rect);
    let eta = (east - FALSE_EASTING) / (K0 * a_rect);

    let beta = [
        n / 2.0 - 2.0 * n2 / 3.0 + 37.0 * n3 / 96.0,
        n2 / 48.0 + n3 / 15.0,
        17.0 * n3 / 480.0,
    ];
    let mut xi_p = xi;
    let mut eta_p = eta;
    for (j, b) in beta.iter().enumerate() {
        let k = 2.0 * (j as f64 + 1.0);
        xi_p -= b * (k * xi).sin() * (k * eta).cosh();
        eta_p -= b * (k * xi).cos() * (k * eta).sinh();
    }

    let chi = (xi_p.sin() / eta_p.cosh()).asin();
    let delta = [
        2.0 * n - 2.0 * n2 / 3.0 - 2.0 * n3,
        7.0 * n2 / 3.0 - 8.0 * n3 / 5.0,
        56.0 * n3 / 15.0,
    ];
    let mut phi = chi;
    for (j, d) in delta.iter().enumerate() {
        let k = 2.0 * (j as f64 + 1.0);
        phi += d * (k * chi).sin();
    }

    let lon0 = ((zone as f64 - 1.0) * 6.0 - 180.0 + 3.0).to_radians();
    let lambda = lon0 + (eta_p.sinh() / xi_p.cos()).atan();

    (phi.to_degrees(), lambda.to_degrees())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference values from PROJ (pyproj, EPSG:25832 → EPSG:4326).
    /// Tolerance 5e-7° ≈ 5 cm.
    #[test]
    fn matches_proj_reference_points() {
        let cases = [
            // Lüdinghausen centroid (from the cadastre import)
            (391_917.0, 5_736_400.0, 51.768_039_02, 7.433_643_27),
            // Bochum, first row of the NRW export
            (370_279.838, 5_705_335.366, 51.484_274_86, 7.131_745_31),
            // NE corner of the Lüdinghausen bounding box
            (399_048.219, 5_743_354.776, 51.831_882_19, 7.534_927_32),
            // SW corner
            (381_287.475, 5_731_482.148, 51.721_687_88, 7.281_348_33),
            // Central meridian: longitude must be exactly 9°E
            (500_000.0, 5_500_000.0, 49.652_542_92, 9.0),
        ];
        for (e, n, lat, lon) in cases {
            let (got_lat, got_lon) = utm32n_to_wgs84(e, n);
            assert!(
                (got_lat - lat).abs() < 5e-7,
                "lat for ({e},{n}): got {got_lat}, want {lat}"
            );
            assert!(
                (got_lon - lon).abs() < 5e-7,
                "lon for ({e},{n}): got {got_lon}, want {lon}"
            );
        }
    }
}
