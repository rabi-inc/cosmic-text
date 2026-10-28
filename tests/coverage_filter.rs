use cosmic_text::{fontdb, Attrs, Buffer, Family, FontSystem, Metrics, Shaping};

fn font_system() -> FontSystem {
    let mut db = fontdb::Database::new();
    db.load_fonts_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts"));
    FontSystem::new_with_locale_and_db("en-US".into(), db)
}

/// `(face, glyph id)` of each glyph of `text` shaped with `family` first.
fn shaped(family: &str, text: &str) -> Vec<(String, u16)> {
    let mut font_system = font_system();
    let mut buffer = Buffer::new(&mut font_system, Metrics::new(14.0, 20.0));
    buffer.set_text(
        text,
        &Attrs::new().family(Family::Name(family)),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(&mut font_system, false);
    let runs: Vec<_> = buffer
        .layout_runs()
        .flat_map(|run| run.glyphs.to_vec())
        .collect();
    runs.into_iter()
        .map(|glyph| {
            let face = font_system
                .db()
                .face(glyph.font_id)
                .map(|face| face.post_script_name.clone())
                .unwrap_or_default();
            (face, glyph.glyph_id)
        })
        .collect()
}

/// Skipping fallback fonts that map none of the missing characters keeps the fonts and glyphs
/// fallback picks. The expectation was recorded with every fallback font shaped (no skipping);
/// here `Noto Sans Hebrew` is skipped as a fallback for the Cyrillic and Greek clusters.
#[test]
fn fallback_picks_the_same_fonts_with_or_without_the_coverage_filter() {
    let got = shaped("Noto Sans Hebrew", "Привет αβγ");
    assert!(got.iter().all(|(_, id)| *id != 0), "{got:?}");
    assert_eq!(got, expected());
}

fn expected() -> Vec<(String, u16)> {
    [
        ("NotoSans-Regular", 439),
        ("NotoSans-Regular", 472),
        ("NotoSans-Regular", 464),
        ("NotoSans-Regular", 458),
        ("NotoSans-Regular", 461),
        ("NotoSans-Regular", 474),
        ("NotoSansHebrew-Regular", 3),
        ("NotoSans-Regular", 381),
        ("NotoSans-Regular", 382),
        ("NotoSans-Regular", 383),
    ]
    .into_iter()
    .map(|(face, id)| (face.to_string(), id))
    .collect()
}
