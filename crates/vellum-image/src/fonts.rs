//! the fonts text layers can use: the system's, plus any downloaded from google fonts into
//! vellum's own font folder.
//!
//! one library is shared by every document. it is built the first time text is drawn,
//! which scans the system fonts once (a few hundred milliseconds); downloaded fonts are
//! added to it as they arrive. text layers name fonts by family, weight and italic, so a
//! document opened on a machine without its fonts still knows what it wants: the layer
//! keeps its last drawn pixels, is marked as missing a font, and can be pointed at another
//! family or have the original fetched from google fonts.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// a font a text layer asks for
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FontRef {
    pub family: String,
    #[serde(default = "regular")]
    pub weight: u16,
    #[serde(default)]
    pub italic: bool,
}

fn regular() -> u16 {
    400
}

impl Default for FontRef {
    fn default() -> Self {
        Self {
            family: "sans-serif".into(),
            weight: 400,
            italic: false,
        }
    }
}

/// a variable font's axis, eg weight from 100 to 900
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FontAxis {
    pub tag: String,
    pub name: String,
    pub min: f32,
    pub default: f32,
    pub max: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FontStyle {
    pub weight: u16,
    pub italic: bool,
    pub name: String,
}

/// a family as the font browser lists it
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FontFamily {
    pub name: String,
    pub styles: Vec<FontStyle>,
    /// axes when any of its faces is a variable font
    pub axes: Vec<FontAxis>,
    /// opentype features the regular face offers, eg `liga`, `smcp`, `ss01`
    pub features: Vec<String>,
    /// came from google fonts rather than the system
    pub downloaded: bool,
}

/// one loaded face: its file's bytes and which face in the file
#[derive(Clone)]
pub struct FaceData {
    pub data: Arc<Vec<u8>>,
    pub index: u32,
}

pub struct FontLibrary {
    db: fontdb::Database,
    folder: Option<PathBuf>,
}

/// where downloaded fonts live, set at start-up
static FOLDER: OnceLock<PathBuf> = OnceLock::new();

/// the library, built on first use: scanning the system's fonts takes a moment, so it
/// never happens while the app starts
fn library() -> &'static RwLock<FontLibrary> {
    static LIB: OnceLock<RwLock<FontLibrary>> = OnceLock::new();
    LIB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let folder = FOLDER.get().cloned();
        if let Some(dir) = &folder
            && std::fs::create_dir_all(dir).is_ok()
        {
            db.load_fonts_dir(dir);
        }
        RwLock::new(FontLibrary { db, folder })
    })
}

/// where downloaded fonts are kept. call once at start-up; it does not scan anything.
pub fn set_font_folder(folder: &Path) {
    let _ = FOLDER.set(folder.to_owned());
}

fn style_name(weight: u16, italic: bool) -> String {
    let w = match weight {
        0..=150 => "Thin",
        151..=250 => "Extra Light",
        251..=350 => "Light",
        351..=450 => "Regular",
        451..=550 => "Medium",
        551..=650 => "Semibold",
        651..=750 => "Bold",
        751..=850 => "Extra Bold",
        _ => "Black",
    };
    match (w, italic) {
        ("Regular", true) => "Italic".into(),
        (w, true) => format!("{w} Italic"),
        (w, false) => w.into(),
    }
}

/// every family, sorted by name
pub fn families() -> Vec<FontFamily> {
    let lib = library().read();
    let mut map: std::collections::BTreeMap<String, FontFamily> = Default::default();
    for face in lib.db.faces() {
        let Some((name, _)) = face.families.first() else {
            continue;
        };
        let downloaded = matches!(
            (&face.source, &lib.folder),
            (fontdb::Source::File(p), Some(dir)) if p.starts_with(dir)
        );
        let entry = map.entry(name.clone()).or_insert_with(|| FontFamily {
            name: name.clone(),
            styles: Vec::new(),
            axes: Vec::new(),
            features: Vec::new(),
            downloaded,
        });
        let weight = face.weight.0;
        let italic = face.style != fontdb::Style::Normal;
        if !entry
            .styles
            .iter()
            .any(|s| s.weight == weight && s.italic == italic)
        {
            entry.styles.push(FontStyle {
                weight,
                italic,
                name: style_name(weight, italic),
            });
        }
        if entry.axes.is_empty() || entry.features.is_empty() {
            lib.db.with_face_data(face.id, |data, index| {
                if let Ok(f) = ttf_parser::Face::parse(data, index) {
                    if entry.axes.is_empty() {
                        entry.axes = axes_of(&f);
                    }
                    if entry.features.is_empty() {
                        entry.features = features_of(&f);
                    }
                }
            });
        }
    }
    let mut out: Vec<FontFamily> = map.into_values().collect();
    for f in &mut out {
        f.styles.sort_by_key(|s| (s.weight, s.italic));
    }
    out
}

fn axes_of(face: &ttf_parser::Face) -> Vec<FontAxis> {
    face.variation_axes()
        .into_iter()
        .map(|a| {
            let tag = a.tag.to_string();
            let name = match tag.as_str() {
                "wght" => "Weight",
                "wdth" => "Width",
                "slnt" => "Slant",
                "ital" => "Italic",
                "opsz" => "Optical size",
                _ => "Axis",
            };
            FontAxis {
                name: if name == "Axis" {
                    tag.clone()
                } else {
                    name.into()
                },
                tag,
                min: a.min_value,
                default: a.def_value,
                max: a.max_value,
            }
        })
        .collect()
}

fn features_of(face: &ttf_parser::Face) -> Vec<String> {
    let mut out: Vec<String> = face
        .tables()
        .gsub
        .iter()
        .flat_map(|t| t.features.into_iter())
        .map(|f| f.tag.to_string())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// the face for a font, or none when the family is not installed
pub fn find(font: &FontRef) -> Option<FaceData> {
    let lib = library().read();
    let families = [fontdb::Family::Name(&font.family)];
    let generic;
    let families: &[fontdb::Family] = match font.family.as_str() {
        "sans-serif" => {
            generic = [fontdb::Family::SansSerif];
            &generic
        }
        "serif" => {
            generic = [fontdb::Family::Serif];
            &generic
        }
        "monospace" => {
            generic = [fontdb::Family::Monospace];
            &generic
        }
        _ => &families,
    };
    let query = fontdb::Query {
        families,
        weight: fontdb::Weight(font.weight),
        stretch: fontdb::Stretch::Normal,
        style: if font.italic {
            fontdb::Style::Italic
        } else {
            fontdb::Style::Normal
        },
    };
    let id = lib.db.query(&query)?;
    // fontdb picks the closest family member; a different family means it is missing
    if !matches!(families[0], fontdb::Family::Name(_))
        || lib.db.face(id).is_some_and(|f| {
            f.families
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case(&font.family))
        })
    {
        return lib.db.with_face_data(id, |data, index| FaceData {
            data: Arc::new(data.to_vec()),
            index,
        });
    }
    None
}

/// the face to draw with: the font asked for, or the default sans serif with a flag
/// saying the real one is missing
pub fn resolve(font: &FontRef) -> Option<(FaceData, bool)> {
    if let Some(f) = find(font) {
        return Some((f, false));
    }
    let fallback = FontRef {
        family: "sans-serif".into(),
        ..font.clone()
    };
    find(&fallback).map(|f| (f, true))
}

/// adds font files to the library, eg after a download
pub fn add_files(paths: &[PathBuf]) {
    let mut lib = library().write();
    for p in paths {
        if let Err(err) = lib.db.load_font_file(p) {
            tracing::warn!(%err, path = %p.display(), "a font could not be loaded");
        }
    }
}

// ---- google fonts ----

/// a family in the google fonts catalogue
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GoogleFont {
    pub family: String,
    pub category: String,
    /// eg "400", "700i"
    pub variants: Vec<String>,
}

const CATALOGUE_URL: &str = "https://fonts.google.com/metadata/fonts";

/// reads the catalogue google serves, which starts with a line guarding against json
/// hijacking
pub fn parse_catalogue(text: &str) -> Result<Vec<GoogleFont>, String> {
    let json = text.trim_start().trim_start_matches(")]}'").trim_start();
    let v: serde_json::Value = serde_json::from_str(json)
        .map_err(|err| format!("The font list could not be read: {err}"))?;
    let list = v
        .get("familyMetadataList")
        .and_then(|l| l.as_array())
        .ok_or("The font list is not in the expected shape.")?;
    let mut out: Vec<GoogleFont> = list
        .iter()
        .filter_map(|f| {
            let family = f.get("family")?.as_str()?.to_owned();
            let category = f
                .get("category")
                .and_then(|c| c.as_str())
                .unwrap_or("Sans Serif")
                .to_owned();
            let variants = f
                .get("fonts")
                .and_then(|m| m.as_object())
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_else(|| vec!["400".to_owned()]);
            Some(GoogleFont {
                family,
                category,
                variants,
            })
        })
        .collect();
    out.sort_by(|a, b| a.family.cmp(&b.family));
    Ok(out)
}

/// the css google serves for a family, asking for every listed variant. without a modern
/// browser's user agent the css points at plain truetype files, which is what we want.
pub fn css_url(font: &GoogleFont) -> String {
    let mut plain: Vec<u16> = Vec::new();
    let mut italic: Vec<u16> = Vec::new();
    for v in &font.variants {
        let (num, it) = match v.strip_suffix('i') {
            Some(n) => (n, true),
            None => (v.as_str(), false),
        };
        if let Ok(w) = num.parse::<u16>() {
            if it {
                italic.push(w);
            } else {
                plain.push(w);
            }
        }
    }
    plain.sort_unstable();
    italic.sort_unstable();
    let family = font.family.replace(' ', "+");
    if italic.is_empty() {
        let ws: Vec<String> = plain.iter().map(u16::to_string).collect();
        if ws.is_empty() {
            return format!("https://fonts.googleapis.com/css2?family={family}");
        }
        return format!(
            "https://fonts.googleapis.com/css2?family={family}:wght@{}",
            ws.join(";")
        );
    }
    let mut pairs: Vec<String> = plain.iter().map(|w| format!("0,{w}")).collect();
    pairs.extend(italic.iter().map(|w| format!("1,{w}")));
    format!(
        "https://fonts.googleapis.com/css2?family={family}:ital,wght@{}",
        pairs.join(";")
    )
}

/// the font file urls in google's css, with the weight and style each is for
pub fn parse_css(css: &str) -> Vec<(String, u16, bool)> {
    let mut out: Vec<(String, u16, bool)> = Vec::new();
    for block in css.split("@font-face").skip(1) {
        let style = block.contains("font-style: italic");
        let weight = block
            .split("font-weight:")
            .nth(1)
            .and_then(|r| r.trim_start().split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|w| w.parse().ok())
            .unwrap_or(400);
        if let Some(url) = block.split("url(").nth(1).and_then(|r| r.split(')').next()) {
            let url = url.trim_matches(|c| c == '"' || c == '\'').to_owned();
            if !out.iter().any(|(u, ..)| *u == url) {
                out.push((url, weight, style));
            }
        }
    }
    out
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(60)))
        .user_agent("Vellum")
        .build()
        .into()
}

fn get(url: &str) -> Result<Vec<u8>, String> {
    let mut res = agent().get(url).call().map_err(|err| {
        format!("Google Fonts could not be reached ({err}). Check the connection and try again.")
    })?;
    res.body_mut()
        .with_config()
        .limit(64 << 20)
        .read_to_vec()
        .map_err(|err| format!("The download stopped part way: {err}"))
}

/// fetches the google fonts catalogue, caching it in `cache` for a week
pub fn google_catalogue(cache: &Path) -> Result<Vec<GoogleFont>, String> {
    let fresh = std::fs::metadata(cache)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age.as_secs() < 7 * 24 * 3600);
    if fresh && let Ok(text) = std::fs::read_to_string(cache) {
        return parse_catalogue(&text);
    }
    let bytes = get(CATALOGUE_URL)?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let list = parse_catalogue(&text)?;
    if let Some(dir) = cache.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(cache, text);
    Ok(list)
}

/// downloads every style of a google font into the font folder and loads it. `progress`
/// is told how many files are done out of how many.
pub fn download_google(
    font: &GoogleFont,
    mut progress: impl FnMut(usize, usize) -> bool,
) -> Result<Vec<PathBuf>, String> {
    let folder = library()
        .read()
        .folder
        .clone()
        .ok_or("Vellum has nowhere to keep downloaded fonts.")?;
    let css = String::from_utf8_lossy(&get(&css_url(font))?).into_owned();
    let files = parse_css(&css);
    if files.is_empty() {
        return Err(format!("Google Fonts sent no files for {}.", font.family));
    }
    let stem: String = font
        .family
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let mut saved = Vec::new();
    for (i, (url, weight, italic)) in files.iter().enumerate() {
        if !progress(i, files.len()) {
            return Err("The download was cancelled.".into());
        }
        let ext = if url.ends_with(".otf") { "otf" } else { "ttf" };
        let path = folder.join(format!(
            "{stem}-{weight}{}.{ext}",
            if *italic { "i" } else { "" }
        ));
        let bytes = get(url)?;
        ttf_parser::Face::parse(&bytes, 0).map_err(|_| {
            format!(
                "Google Fonts sent a file for {} that is not a font.",
                font.family
            )
        })?;
        std::fs::write(&path, &bytes)
            .map_err(|err| format!("The font could not be saved: {err}"))?;
        saved.push(path);
    }
    progress(files.len(), files.len());
    add_files(&saved);
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogue_and_css_parse() {
        let text = r#")]}'
{"familyMetadataList":[{"family":"Roboto Slab","category":"Serif","fonts":{"400":{},"700":{}}},{"family":"Inter","category":"Sans Serif","fonts":{"400":{},"400i":{}}}]}"#;
        let list = parse_catalogue(text).expect("parse");
        assert_eq!(list[0].family, "Inter");
        assert_eq!(
            css_url(&list[1]),
            "https://fonts.googleapis.com/css2?family=Roboto+Slab:wght@400;700"
        );
        assert!(css_url(&list[0]).contains("ital,wght@0,400;1,400"));
        let css = "/* latin */\n@font-face {\n  font-family: 'Inter';\n  font-style: italic;\n  font-weight: 400;\n  src: url(https://fonts.gstatic.com/s/inter/v1/a.ttf) format('truetype');\n}\n@font-face {\n  font-family: 'Inter';\n  font-style: normal;\n  font-weight: 700;\n  src: url(https://fonts.gstatic.com/s/inter/v1/b.ttf) format('truetype');\n}\n";
        let files = parse_css(css);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].1, 400);
        assert!(files[0].2 && !files[1].2);
    }

    #[test]
    fn styles_have_names() {
        assert_eq!(style_name(400, false), "Regular");
        assert_eq!(style_name(400, true), "Italic");
        assert_eq!(style_name(700, true), "Bold Italic");
    }

    #[test]
    fn system_fonts_are_found_or_missing_is_reported() {
        if families().is_empty() {
            return;
        }
        assert!(resolve(&FontRef::default()).is_some());
        let missing = FontRef {
            family: "No Such Family Anywhere".into(),
            ..Default::default()
        };
        assert!(find(&missing).is_none());
        assert!(resolve(&missing).is_some_and(|(_, m)| m));
    }
}
