//! A headless worker can start without walking the system font folders (OC-33 E1). Set before the
//! worker starts, `filmcraft_engine::graphics::set_system_scan(false)` keeps every font lookup to
//! the bundled and registered fonts: `fonts.list` (which scans by default), a title or caption in
//! a family that is not registered (it falls back to Inter, as before) and a direct
//! `scan_system` call read nothing from disk. The default is unchanged: the scan is on, and
//! switched back on it runs as before. `filmcraft_text::fonts::scan_system` always walked the
//! folders on the first lookup of a family it did not know.

use filmcraft_engine::Session;
use filmcraft_engine::graphics::{set_system_scan, system_scan_enabled};
use filmcraft_text::fonts;
use serde_json::json;

fn system_faces() -> usize {
    fonts::all_faces().iter().filter(|f| f.info.origin == "system").count()
}

/// One test, its steps in order: the switch and the font database are process-wide.
#[test]
fn a_worker_can_skip_the_system_font_scan() {
    assert!(system_scan_enabled(), "the scan is on by default");
    set_system_scan(false);
    assert!(!system_scan_enabled());
    let mut s = Session::default();
    let listed = s.execute("fonts.list", json!({})).unwrap();
    assert!(listed.as_array().is_some_and(|a| a.iter().any(|f| f["family"] == "Inter")), "the bundled fonts are listed: {listed}");
    assert_eq!(fonts::scan_system(), 0, "a scan call reads nothing");
    let r = filmcraft_text::resolve("No Such Family For The Worker", "Regular");
    assert!(r.missing, "an unregistered family falls back");
    assert_eq!(fonts::face(r.face).info.family, "Inter");
    assert!(!fonts::system_scanned(), "the system font folders were not scanned");
    assert_eq!(system_faces(), 0, "no system face was added");

    set_system_scan(true);
    assert!(system_scan_enabled());
    s.execute("fonts.list", json!({})).unwrap();
    assert!(fonts::system_scanned(), "switched back on, fonts.list scans as before");
}
