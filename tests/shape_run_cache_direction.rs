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

/// Adding fonts drops cached runs: a run that missed every glyph before is shaped again with the
/// font that now has them.
#[test]
fn changing_the_font_set_drops_cached_runs() {
    let mut db = fontdb::Database::new();
    let fonts = concat!(env!("CARGO_MANIFEST_DIR"), "/fonts");
    db.load_font_file(format!("{fonts}/NotoSansHebrew.ttf"))
        .unwrap();
    let mut font_system = FontSystem::new_with_locale_and_db("en-US".into(), db);
    let shape = |font_system: &mut FontSystem| {
        let mut buffer = Buffer::new(font_system, Metrics::new(14.0, 20.0));
        buffer.set_text(
            "Привет",
            &Attrs::new().family(Family::Name("Noto Sans Hebrew")),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(font_system, false);
        buffer
            .layout_runs()
            .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.glyph_id))
            .collect::<Vec<_>>()
    };
    assert!(shape(&mut font_system).iter().all(|id| *id == 0));
    font_system
        .db_mut()
        .load_font_file(format!("{fonts}/NotoSans-Regular.ttf"))
        .unwrap();
    assert!(shape(&mut font_system).iter().all(|id| *id != 0));
}
