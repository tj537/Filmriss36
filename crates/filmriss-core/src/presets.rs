//! Eingebaute Filmsimulationen.
//!
//! Die Rezepte liegen als TOML in `crates/filmriss-core/presets/` und werden zur
//! Compile-Zeit eingebettet. Sie dienen gleichzeitig als Vorlage für eigene Rezepte.
//!
//! Hinweis: Die Namen sind bewusst eigene. Fujifilm-Simulationsnamen sind Marken,
//! und Fujifilms Original-LUTs sind nicht öffentlich – die Looks hier sind eigene
//! Interpretationen, die sich an den bekannten Charakteristika orientieren.

use crate::recipe::FilmRecipe;

const PRESETS: &[(&str, &str)] = &[
    ("standard", include_str!("../presets/01_standard.toml")),
    ("velvet-dia", include_str!("../presets/02_velvet_dia.toml")),
    ("sanftes-portrait", include_str!("../presets/03_sanftes_portrait.toml")),
    ("chrome-doku", include_str!("../presets/04_chrome_doku.toml")),
    ("negativ-klassik", include_str!("../presets/05_negativ_klassik.toml")),
    ("negativ-portrait", include_str!("../presets/06_negativ_portrait.toml")),
    ("kino-flach", include_str!("../presets/07_kino_flach.toml")),
    ("bleach-bypass", include_str!("../presets/08_bleach_bypass.toml")),
    ("nostalgie", include_str!("../presets/09_nostalgie.toml")),
    ("natur", include_str!("../presets/10_natur.toml")),
    ("sommer-gold", include_str!("../presets/11_sommer_gold.toml")),
    ("teal-orange", include_str!("../presets/12_teal_orange.toml")),
    ("mono-fein", include_str!("../presets/13_mono_fein.toml")),
    ("mono-rotfilter", include_str!("../presets/14_mono_rotfilter.toml")),
    ("sepia", include_str!("../presets/15_sepia.toml")),
];

/// Alle eingebauten Rezepte (id, Rezept) in Anzeigereihenfolge.
pub fn builtin() -> Vec<(&'static str, FilmRecipe)> {
    PRESETS
        .iter()
        .map(|(id, text)| {
            let recipe =
                FilmRecipe::from_toml(text).unwrap_or_else(|e| panic!("eingebautes Rezept '{id}' ist ungültig: {e}"));
            (*id, recipe)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn all_presets_parse_and_bake() {
        let presets = super::builtin();
        assert_eq!(presets.len(), 15);
        for (id, recipe) in presets {
            let lut = recipe.bake(9, None);
            for c in lut.data() {
                assert!(
                    c.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
                    "{id}: {c:?}"
                );
            }
        }
    }
}
