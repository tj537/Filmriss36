//! Built-in film simulations.
//!
//! The recipes live as TOML in `crates/filmriss-core/presets/` and are embedded at
//! compile time. They also serve as templates for custom recipes.
//!
//! Note: the names are deliberately our own. Fujifilm's simulation names are trademarks
//! and Fujifilm's original LUTs are not public – the looks here are our own
//! interpretations based on the well-known characteristics.

use crate::recipe::FilmRecipe;

const PRESETS: &[(&str, &str)] = &[
    ("standard", include_str!("../presets/01_standard.toml")),
    ("velvet-slide", include_str!("../presets/02_velvet_slide.toml")),
    ("soft-portrait", include_str!("../presets/03_soft_portrait.toml")),
    ("chrome-docu", include_str!("../presets/04_chrome_docu.toml")),
    ("classic-negative", include_str!("../presets/05_classic_negative.toml")),
    (
        "portrait-negative",
        include_str!("../presets/06_portrait_negative.toml"),
    ),
    ("flat-cinema", include_str!("../presets/07_flat_cinema.toml")),
    ("bleach-bypass", include_str!("../presets/08_bleach_bypass.toml")),
    ("nostalgia", include_str!("../presets/09_nostalgia.toml")),
    ("natural", include_str!("../presets/10_natural.toml")),
    ("summer-gold", include_str!("../presets/11_summer_gold.toml")),
    ("teal-orange", include_str!("../presets/12_teal_orange.toml")),
    ("mono-fine", include_str!("../presets/13_mono_fine.toml")),
    ("mono-red-filter", include_str!("../presets/14_mono_red_filter.toml")),
    ("sepia", include_str!("../presets/15_sepia.toml")),
];

/// All built-in recipes (id, recipe) in display order.
pub fn builtin() -> Vec<(&'static str, FilmRecipe)> {
    PRESETS
        .iter()
        .map(|(id, text)| {
            let recipe =
                FilmRecipe::from_toml(text).unwrap_or_else(|e| panic!("built-in recipe '{id}' is invalid: {e}"));
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
