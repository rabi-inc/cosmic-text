// SPDX-License-Identifier: MIT OR Apache-2.0

use harfrust::Shaper;
use linebender_resource_handle::{Blob, FontData};
use skrifa::raw::{ReadError, TableProvider as _};
use skrifa::{metrics::Metrics, prelude::*};
// re-export skrifa
pub use skrifa;
// re-export peniko::Font;
#[cfg(feature = "peniko")]
pub use linebender_resource_handle::FontData as PenikoFont;

use core::fmt;

use alloc::sync::Arc;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use fontdb::Style;
use self_cell::self_cell;

/// Code point ranges that cover every character the shaper can map from the character map.
///
/// The union of the Unicode subtables in format 4 and 12, so it holds whichever of them the shaper
/// picks. The ranges may cover characters the font does not map; they never miss one it maps.
/// `None` when the cmap has a subtable the shaper may read that is not bounded here: a symbol or
/// Mac Roman subtable (the shaper remaps characters into them) or another format.
fn cmap_ranges(cmap: &skrifa::raw::tables::cmap::Cmap<'_>) -> Option<Vec<(u32, u32)>> {
    use skrifa::raw::tables::cmap::{CmapSubtable, PlatformId};

    let data = cmap.offset_data();
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    for record in cmap.encoding_records() {
        let symbol = record.platform_id() == PlatformId::Windows && record.encoding_id() == 0;
        if symbol || record.platform_id() == PlatformId::Macintosh {
            return None;
        }
        match record.subtable(data).ok()? {
            CmapSubtable::Format4(table) => ranges.extend(
                table
                    .start_code()
                    .iter()
                    .zip(table.end_code())
                    .map(|(start, end)| (u32::from(start.get()), u32::from(end.get()))),
            ),
            CmapSubtable::Format12(table) => ranges.extend(
                table
                    .groups()
                    .iter()
                    .map(|group| (group.start_char_code(), group.end_char_code())),
            ),
            // Variation sequences resolve to a base character the other subtables map.
            CmapSubtable::Format14(_) => {}
            _ => return None,
        }
    }
    ranges.sort_unstable();
    // Merge overlapping ranges so a binary search finds the one that holds a character.
    let mut merged: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        match merged.last_mut() {
            Some(last) if start <= last.1.saturating_add(1) => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    Some(merged)
}

#[cfg(test)]
mod cmap_ranges_tests {
    use super::cmap_ranges;
    use skrifa::raw::{tables::cmap::Cmap, FontRead};

    fn u16s(values: &[u16]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    fn u32s(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    /// Format 4 with one segment `start..=end` and the closing 0xFFFF segment.
    fn format4(start: u16, end: u16) -> Vec<u8> {
        let mut body = u16s(&[4, 32, 0, 4, 4, 1, 0]);
        body.extend(u16s(&[end, 0xFFFF, 0, start, 0xFFFF, 1, 1, 0, 0]));
        body
    }

    /// Format 12 or 13 with one group `start..=end`.
    fn format12(format: u16, start: u32, end: u32) -> Vec<u8> {
        let mut body = u16s(&[format, 0]);
        body.extend(u32s(&[28, 0, 1, start, end, 1]));
        body
    }

    fn format0() -> Vec<u8> {
        let mut body = u16s(&[0, 262, 0]);
        body.extend([0u8; 256]);
        body
    }

    fn cmap(subtables: &[((u16, u16), Vec<u8>)]) -> Vec<u8> {
        let mut out = u16s(&[0, subtables.len() as u16]);
        let mut offset = 4 + 8 * subtables.len() as u32;
        for ((platform, encoding), body) in subtables {
            out.extend(u16s(&[*platform, *encoding]));
            out.extend(u32s(&[offset]));
            offset += body.len() as u32;
        }
        for (_, body) in subtables {
            out.extend(body);
        }
        out
    }

    fn ranges(subtables: &[((u16, u16), Vec<u8>)]) -> Option<Vec<(u32, u32)>> {
        let bytes = cmap(subtables);
        let cmap = Cmap::read(bytes.as_slice().into()).expect("cmap");
        cmap_ranges(&cmap)
    }

    #[test]
    fn unicode_subtables_give_the_union_of_their_ranges() {
        assert_eq!(
            ranges(&[
                ((3, 1), format4(0x41, 0x5A)),
                ((3, 10), format12(12, 0x1_F600, 0x1_F64F)),
            ]),
            Some(vec![(0x41, 0x5A), (0xFFFF, 0xFFFF), (0x1_F600, 0x1_F64F)])
        );
    }

    #[test]
    fn subtables_the_shaper_remaps_or_reads_otherwise_are_unbounded() {
        assert_eq!(ranges(&[((0, 4), format12(13, 0, 0x10_FFFF))]), None);
        assert_eq!(
            ranges(&[
                ((3, 0), format4(0xF020, 0xF0FF)),
                ((3, 1), format4(0x41, 0x5A))
            ]),
            None
        );
        assert_eq!(ranges(&[((1, 0), format0())]), None);
    }
}

pub mod fallback;
pub use fallback::{Fallback, PlatformFallback};

pub use self::system::*;
mod system;

struct OwnedFaceData {
    data: Arc<dyn AsRef<[u8]> + Send + Sync>,
    shaper_data: harfrust::ShaperData,
    shaper_instance: harfrust::ShaperInstance,
    metrics: Metrics,
}

self_cell!(
    struct OwnedFace {
        owner: OwnedFaceData,

        #[covariant]
        dependent: Shaper,
    }
);

struct FontMonospaceFallback {
    monospace_em_width: Option<f32>,
    scripts: Vec<[u8; 4]>,
    unicode_codepoints: Vec<u32>,
}

/// A font
pub struct Font {
    #[cfg(feature = "swash")]
    swash: (u32, swash::CacheKey),
    harfrust: OwnedFace,
    data: FontData,
    id: fontdb::ID,
    monospace_fallback: Option<FontMonospaceFallback>,
    /// Sorted, disjoint code point ranges that cover every character the shaper can map, or `None`
    /// when they cannot be bounded (see [`cmap_ranges`]).
    cmap_ranges: Option<Vec<(u32, u32)>>,
    pub(crate) italic_or_oblique: bool,
}

impl fmt::Debug for Font {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Font")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Font {
    pub const fn id(&self) -> fontdb::ID {
        self.id
    }

    pub fn monospace_em_width(&self) -> Option<f32> {
        self.monospace_fallback
            .as_ref()
            .and_then(|x| x.monospace_em_width)
    }

    pub fn scripts(&self) -> &[[u8; 4]] {
        self.monospace_fallback.as_ref().map_or(&[], |x| &x.scripts)
    }

    pub fn unicode_codepoints(&self) -> &[u32] {
        self.monospace_fallback
            .as_ref()
            .map_or(&[], |x| &x.unicode_codepoints)
    }

    pub fn data(&self) -> &[u8] {
        self.data.data.data()
    }

    /// Whether the shaper may find a glyph for `c` in the character map of the font. `false` only
    /// when the character is outside every range of the subtable the shaper reads.
    pub(crate) fn may_map(&self, c: char) -> bool {
        self.cmap_ranges.as_ref().is_none_or(|ranges| {
            let c = u32::from(c);
            ranges
                .binary_search_by(|&(start, end)| {
                    if end < c {
                        core::cmp::Ordering::Less
                    } else if start > c {
                        core::cmp::Ordering::Greater
                    } else {
                        core::cmp::Ordering::Equal
                    }
                })
                .is_ok()
        })
    }

    pub fn shaper(&self) -> &harfrust::Shaper<'_> {
        self.harfrust.borrow_dependent()
    }

    pub(crate) fn shaper_instance(&self) -> &harfrust::ShaperInstance {
        &self.harfrust.borrow_owner().shaper_instance
    }

    pub fn metrics(&self) -> &Metrics {
        &self.harfrust.borrow_owner().metrics
    }

    #[cfg(feature = "peniko")]
    pub fn as_peniko(&self) -> PenikoFont {
        self.data.clone()
    }

    #[cfg(feature = "swash")]
    pub fn as_swash(&self) -> swash::FontRef<'_> {
        let swash = &self.swash;
        swash::FontRef {
            data: self.data(),
            offset: swash.0,
            key: swash.1,
        }
    }
}

impl Font {
    pub fn new(db: &fontdb::Database, id: fontdb::ID, weight: fontdb::Weight) -> Option<Self> {
        let info = db.face(id)?;

        let data = match &info.source {
            fontdb::Source::Binary(data) => Arc::clone(data),
            #[cfg(feature = "std")]
            fontdb::Source::File(path) => {
                log::warn!("Unsupported fontdb Source::File('{}')", path.display());
                return None;
            }
            #[cfg(feature = "std")]
            fontdb::Source::SharedFile(_path, data) => Arc::clone(data),
        };

        // It's a bit unfortunate but we need to parse the data into a `FontRef`
        // twice--once to construct the HarfRust `ShaperInstance` and
        // `ShaperData`, and once to create the persistent `FontRef` tied to the
        // lifetime of the face data.
        let font_ref = FontRef::from_index((*data).as_ref(), info.index).ok()?;
        let location = font_ref
            .axes()
            .location([(Tag::new(b"wght"), weight.0 as f32)]);
        let metrics = font_ref.metrics(Size::unscaled(), &location);

        let monospace_fallback = if cfg!(feature = "monospace_fallback") {
            (|| {
                let glyph_metrics = font_ref.glyph_metrics(Size::unscaled(), &location);
                let charmap = font_ref.charmap();
                let monospace_em_width = info
                    .monospaced
                    .then(|| {
                        let hor_advance = glyph_metrics.advance_width(charmap.map(' ')?)?;
                        let upem = metrics.units_per_em;
                        Some(hor_advance / f32::from(upem))
                    })
                    .flatten();

                if info.monospaced && monospace_em_width.is_none() {
                    None?;
                }

                let scripts = font_ref
                    .gpos()
                    .ok()?
                    .script_list()
                    .ok()?
                    .script_records()
                    .iter()
                    .chain(
                        font_ref
                            .gsub()
                            .ok()?
                            .script_list()
                            .ok()?
                            .script_records()
                            .iter(),
                    )
                    .map(|script| script.script_tag().into_bytes())
                    .collect();

                let mut unicode_codepoints = Vec::new();

                for (code_point, _) in charmap.mappings() {
                    unicode_codepoints.push(code_point);
                }

                unicode_codepoints.shrink_to_fit();

                Some(FontMonospaceFallback {
                    monospace_em_width,
                    scripts,
                    unicode_codepoints,
                })
            })()
        } else {
            None
        };

        let (shaper_instance, shaper_data) = {
            (
                harfrust::ShaperInstance::from_coords(&font_ref, location.coords().iter().copied()),
                harfrust::ShaperData::new(&font_ref),
            )
        };

        let cmap_ranges = font_ref.cmap().ok().and_then(|cmap| cmap_ranges(&cmap));

        Some(Self {
            id: info.id,
            monospace_fallback,
            cmap_ranges,
            #[cfg(feature = "swash")]
            swash: {
                let swash = swash::FontRef::from_index((*data).as_ref(), info.index as usize)?;
                (swash.offset, swash.key)
            },
            harfrust: OwnedFace::try_new(
                OwnedFaceData {
                    data: Arc::clone(&data),
                    shaper_data,
                    shaper_instance,
                    metrics,
                },
                |OwnedFaceData {
                     data,
                     shaper_data,
                     shaper_instance,
                     ..
                 }| {
                    let font_ref = FontRef::from_index((**data).as_ref(), info.index)?;
                    let shaper = shaper_data
                        .shaper(&font_ref)
                        .instance(Some(shaper_instance))
                        .build();
                    Ok::<_, ReadError>(shaper)
                },
            )
            .ok()?,
            data: FontData::new(Blob::new(data), info.index),
            italic_or_oblique: info.style == Style::Italic || info.style == Style::Oblique,
        })
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn test_fonts_load_time() {
        use crate::FontSystem;
        use sys_locale::get_locale;

        #[cfg(not(target_arch = "wasm32"))]
        let now = std::time::Instant::now();

        let mut db = fontdb::Database::new();
        let locale = get_locale().expect("Local available");
        db.load_system_fonts();
        FontSystem::new_with_locale_and_db(locale, db);

        #[cfg(not(target_arch = "wasm32"))]
        println!("Fonts load time {}ms.", now.elapsed().as_millis());
    }
}
