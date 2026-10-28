#![cfg(feature = "shape-run-cache")]

use cosmic_text::{fontdb, Attrs, Buffer, Family, FontSystem, Metrics, Shaping};

fn font_system() -> FontSystem {
    let mut db = fontdb::Database::new();
    db.load_fonts_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts"));
    FontSystem::new_with_locale_and_db("en-US".into(), db)
}

fn glyph_ids(font_system: &mut FontSystem, text: &str) -> Vec<u16> {
    let mut buffer = Buffer::new(font_system, Metrics::new(14.0, 20.0));
    buffer.set_text(
        text,
        &Attrs::new().family(Family::Name("Noto Sans")),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(font_system, false);
    buffer
        .layout_runs()
        .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.glyph_id))
        .collect()
}

/// A bracket shaped in a right-to-left run is mirrored. The cached glyphs of that run must not be
/// reused for the same text in a left-to-right run.
#[test]
fn a_run_cached_in_one_direction_is_not_reused_in_the_other() {
    let mut fresh = font_system();
    let expected = glyph_ids(&mut fresh, "hello (");

    let mut cached = font_system();
    glyph_ids(&mut cached, "שלום (");
    assert_eq!(glyph_ids(&mut cached, "hello ("), expected);
}
