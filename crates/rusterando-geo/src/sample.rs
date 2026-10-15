//! Built-in fixture in the gebref layout, so `geo-import --sample`, the
//! runtime and the tests all work without the 100 MB download.
//!
//! Two municipalities: Lüdinghausen (AGS 05558024, the configured one) and
//! Nordkirchen (05558028, present to prove the AGS filter). Coordinates are
//! EPSG:25832 metres around the real town centre. One row has a suffix, one
//! has a malformed house number.

use crate::config::GeoConfig;

/// Lüdinghausen rows in the fixture that yield an address.
pub const ROWS_KEPT_LH: u64 = 47;
/// Nordkirchen rows in the fixture.
pub const ROWS_KEPT_NK: u64 = 6;
/// Total data rows (kept + Nordkirchen + the malformed one).
pub const ROWS_TOTAL: u64 = ROWS_KEPT_LH + ROWS_KEPT_NK + 1;
/// Distinct Lüdinghausen streets.
pub const STREETS_LH: u64 = 8;

/// The fixture as UTF-8 text (CRLF, like the real export).
pub const SAMPLE_UTF8: &str = concat!(
    // Bahnhofstraße 1..10 + 12, 12a, 12b  (13 rows)
    "N;DENWSAMPLE00001;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00010;Bahnhofstraße;1;;32;391860.000;5736390.000;2026-07-01\r\n",
    "N;DENWSAMPLE00002;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00010;Bahnhofstraße;2;;32;391880.000;5736400.000;2026-07-01\r\n",
    "N;DENWSAMPLE00003;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00010;Bahnhofstraße;3;;32;391900.000;5736410.000;2026-07-01\r\n",
    "N;DENWSAMPLE00004;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00010;Bahnhofstraße;4;;32;391920.000;5736420.000;2026-07-01\r\n",
    "N;DENWSAMPLE00005;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00010;Bahnhofstraße;5;;32;391940.000;5736430.000;2026-07-01\r\n",
    "N;DENWSAMPLE00006;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00010;Bahnhofstraße;6;;32;391960.000;5736440.000;2026-07-01\r\n",
    "N;DENWSAMPLE00007;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00010;Bahnhofstraße;7;;32;391980.000;5736450.000;2026-07-01\r\n",
    "N;DENWSAMPLE00008;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00010;Bahnhofstraße;8;;32;392000.000;5736460.000;2026-07-01\r\n",
    "N;DENWSAMPLE00009;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00010;Bahnhofstraße;9;;32;392020.000;5736470.000;2026-07-01\r\n",
    "N;DENWSAMPLE00010;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00010;Bahnhofstraße;10;;32;392040.000;5736480.000;2026-07-01\r\n",
    "N;DENWSAMPLE00011;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00010;Bahnhofstraße;12;;32;392080.000;5736500.000;2026-07-01\r\n",
    "N;DENWSAMPLE00012;B;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00010;Bahnhofstraße;12;a;32;392090.000;5736505.000;2026-07-01\r\n",
    "N;DENWSAMPLE00013;B;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00010;Bahnhofstraße;12;b;32;392100.000;5736510.000;2026-07-01\r\n",
    // Münsterstraße 1..8 (8 rows)
    "N;DENWSAMPLE00014;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00020;Münsterstraße;1;;32;392100.000;5736600.000;2026-07-01\r\n",
    "N;DENWSAMPLE00015;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00020;Münsterstraße;2;;32;392120.000;5736620.000;2026-07-01\r\n",
    "N;DENWSAMPLE00016;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00020;Münsterstraße;3;;32;392140.000;5736640.000;2026-07-01\r\n",
    "N;DENWSAMPLE00017;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00020;Münsterstraße;4;;32;392160.000;5736660.000;2026-07-01\r\n",
    "N;DENWSAMPLE00018;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00020;Münsterstraße;5;;32;392180.000;5736680.000;2026-07-01\r\n",
    "N;DENWSAMPLE00019;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00020;Münsterstraße;6;;32;392200.000;5736700.000;2026-07-01\r\n",
    "N;DENWSAMPLE00020;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00020;Münsterstraße;7;;32;392220.000;5736720.000;2026-07-01\r\n",
    "N;DENWSAMPLE00021;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00020;Münsterstraße;8;;32;392240.000;5736740.000;2026-07-01\r\n",
    // Hermann-Löns-Straße 1..6 (6 rows, umlaut)
    "N;DENWSAMPLE00022;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00030;Hermann-Löns-Straße;1;;32;391500.000;5736800.000;2026-07-01\r\n",
    "N;DENWSAMPLE00023;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00030;Hermann-Löns-Straße;2;;32;391520.000;5736810.000;2026-07-01\r\n",
    "N;DENWSAMPLE00024;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00030;Hermann-Löns-Straße;3;;32;391540.000;5736820.000;2026-07-01\r\n",
    "N;DENWSAMPLE00025;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00030;Hermann-Löns-Straße;4;;32;391560.000;5736830.000;2026-07-01\r\n",
    "N;DENWSAMPLE00026;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00030;Hermann-Löns-Straße;5;;32;391580.000;5736840.000;2026-07-01\r\n",
    "N;DENWSAMPLE00027;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00030;Hermann-Löns-Straße;6;;32;391600.000;5736850.000;2026-07-01\r\n",
    // Mühlenstraße 1..5 (5 rows, umlaut + ß)
    "N;DENWSAMPLE00028;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00040;Mühlenstraße;1;;32;391700.000;5736200.000;2026-07-01\r\n",
    "N;DENWSAMPLE00029;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00040;Mühlenstraße;2;;32;391720.000;5736210.000;2026-07-01\r\n",
    "N;DENWSAMPLE00030;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00040;Mühlenstraße;3;;32;391740.000;5736220.000;2026-07-01\r\n",
    "N;DENWSAMPLE00031;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00040;Mühlenstraße;4;;32;391760.000;5736230.000;2026-07-01\r\n",
    "N;DENWSAMPLE00032;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00040;Mühlenstraße;5;;32;391780.000;5736240.000;2026-07-01\r\n",
    // Am Markt 1..4 (4 rows)
    "N;DENWSAMPLE00033;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00050;Am Markt;1;;32;391900.000;5736300.000;2026-07-01\r\n",
    "N;DENWSAMPLE00034;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00050;Am Markt;2;;32;391910.000;5736305.000;2026-07-01\r\n",
    "N;DENWSAMPLE00035;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00050;Am Markt;3;;32;391920.000;5736310.000;2026-07-01\r\n",
    "N;DENWSAMPLE00036;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00050;Am Markt;4;;32;391930.000;5736315.000;2026-07-01\r\n",
    // Olfener Straße 10..15 (6 rows)
    "N;DENWSAMPLE00037;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00060;Olfener Straße;10;;32;390500.000;5735500.000;2026-07-01\r\n",
    "N;DENWSAMPLE00038;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00060;Olfener Straße;11;;32;390480.000;5735480.000;2026-07-01\r\n",
    "N;DENWSAMPLE00039;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00060;Olfener Straße;12;;32;390460.000;5735460.000;2026-07-01\r\n",
    "N;DENWSAMPLE00040;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00060;Olfener Straße;13;;32;390440.000;5735440.000;2026-07-01\r\n",
    "N;DENWSAMPLE00041;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00060;Olfener Straße;14;;32;390420.000;5735420.000;2026-07-01\r\n",
    "N;DENWSAMPLE00042;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00060;Olfener Straße;15;;32;390400.000;5735400.000;2026-07-01\r\n",
    // Berenbrock 1..3 (Bauerschaft, 3 rows, far out)
    "N;DENWSAMPLE00043;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00070;Berenbrock;1;;32;386000.000;5739000.000;2026-07-01\r\n",
    "N;DENWSAMPLE00044;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00070;Berenbrock;2;;32;386100.000;5739100.000;2026-07-01\r\n",
    "N;DENWSAMPLE00045;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00070;Berenbrock;3;;32;386200.000;5739200.000;2026-07-01\r\n",
    // Seppenrader Straße 1..2 (2 rows)
    "N;DENWSAMPLE00046;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00080;Seppenrader Straße;1;;32;391000.000;5736900.000;2026-07-01\r\n",
    "N;DENWSAMPLE00047;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00080;Seppenrader Straße;2;;32;391020.000;5736920.000;2026-07-01\r\n",
    // malformed house number (skipped)
    "N;DENWSAMPLE00048;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;024;Lüdinghausen;0000;;00010;Bahnhofstraße;abc;;32;391870.000;5736395.000;2026-07-01\r\n",
    // Nordkirchen (other AGS): Schloßstraße 1..4, Bahnhofstraße 1..2 (6 rows)
    "N;DENWSAMPLE00049;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;028;Nordkirchen;0000;;00010;Schloßstraße;1;;32;398000.000;5728000.000;2026-07-01\r\n",
    "N;DENWSAMPLE00050;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;028;Nordkirchen;0000;;00010;Schloßstraße;2;;32;398020.000;5728020.000;2026-07-01\r\n",
    "N;DENWSAMPLE00051;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;028;Nordkirchen;0000;;00010;Schloßstraße;3;;32;398040.000;5728040.000;2026-07-01\r\n",
    "N;DENWSAMPLE00052;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;028;Nordkirchen;0000;;00010;Schloßstraße;4;;32;398060.000;5728060.000;2026-07-01\r\n",
    "N;DENWSAMPLE00053;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;028;Nordkirchen;0000;;00020;Bahnhofstraße;1;;32;398200.000;5728200.000;2026-07-01\r\n",
    "N;DENWSAMPLE00054;A;05;Nordrhein-Westfalen;5;Münster;58;Coesfeld;028;Nordkirchen;0000;;00020;Bahnhofstraße;2;;32;398220.000;5728220.000;2026-07-01\r\n",
);

/// Tenant config matching the fixture: Lüdinghausen only.
pub fn config() -> GeoConfig {
    let mut vars = std::collections::HashMap::new();
    vars.insert("GEO_BACKEND".to_string(), "cadastre_nrw".to_string());
    vars.insert("GEO_AGS".to_string(), "05558024".to_string());
    vars.insert(
        "GEO_POSTCODE_MAP".to_string(),
        "05558024:59348:Lüdinghausen".to_string(),
    );
    vars.insert(
        "GEO_EXPECT_BBOX".to_string(),
        "51.68,51.86,7.30,7.58".to_string(),
    );
    GeoConfig::from_vars(&vars).expect("sample config")
}

/// The same fixture re-encoded as Windows-1252 (how older exports arrive).
pub fn sample_windows1252() -> Vec<u8> {
    let (bytes, _, had_unmappable) = encoding_rs::WINDOWS_1252.encode(SAMPLE_UTF8);
    assert!(!had_unmappable);
    bytes.into_owned()
}

/// The fixture wrapped in a zip like the real download (`gebref.txt`).
pub fn sample_zip() -> Vec<u8> {
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        w.start_file("gebref.txt", opts).expect("zip entry");
        w.write_all(SAMPLE_UTF8.as_bytes()).expect("zip write");
        w.finish().expect("zip finish");
    }
    buf.into_inner()
}
