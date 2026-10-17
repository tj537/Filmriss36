//! Rezept-Editor: alle Parameter einer Filmsimulation als Regler.
//! Jede Änderung wird sofort neu gebacken (~1 ms) und ist live in der Preview sichtbar.
//! Doppelklick/Doppeltipp auf einen Regler setzt ihn zurück.

use egui::{CollapsingHeader, Color32, RichText, Slider, Ui};
use filmriss_core::FilmRecipe;
use filmriss_core::color::{linear_to_srgb, oklab_to_linear_srgb};
use filmriss_core::recipe::{HueAdjustments, Tone};

use crate::style::TEXT_DIM;

/// Zeigt den Editor; `true`, wenn sich das Rezept geändert hat.
pub fn show(ui: &mut Ui, r: &mut FilmRecipe) -> bool {
    let mut changed = false;

    section(ui, "Belichtung & Weißabgleich", true, |ui| {
        changed |= slider(ui, "Belichtung (EV)", &mut r.exposure, -2.0..=2.0, 0.0);
        changed |= slider(ui, "Temperatur", &mut r.temperature, -1.0..=1.0, 0.0);
        changed |= slider(ui, "Tönung", &mut r.tint, -1.0..=1.0, 0.0);
    });

    section(ui, "Tonwerte", true, |ui| {
        changed |= slider(ui, "Kontrast", &mut r.contrast, -1.0..=1.0, 0.0);
        changed |= slider(ui, "Lichter", &mut r.highlights, -1.0..=1.0, 0.0);
        changed |= slider(ui, "Schatten", &mut r.shadows, -1.0..=1.0, 0.0);
        changed |= slider(ui, "Fade (Schwarz)", &mut r.fade, 0.0..=1.0, 0.0);
    });

    section(ui, "Farbe", true, |ui| {
        changed |= ui.checkbox(&mut r.monochrome, "Schwarzweiß").changed();
        if r.monochrome {
            ui.label(RichText::new("Kanalmischer (Farbfilter)").color(TEXT_DIM).small());
            let default = FilmRecipe::default().mono_mix;
            for (i, name) in ["Rot", "Grün", "Blau"].iter().enumerate() {
                changed |= slider(ui, name, &mut r.mono_mix[i], -0.5..=1.5, default[i]);
            }
        } else {
            changed |= slider(ui, "Sättigung", &mut r.saturation, -1.0..=1.0, 0.0);
        }
    });

    if !r.monochrome {
        section(ui, "Farbbereiche", false, |ui| {
            for (i, band) in r.hues.bands_mut().into_iter().enumerate() {
                let swatch = oklch_color(0.7, 0.13, HueAdjustments::CENTERS[i]);
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 6.0, swatch);
                    ui.strong(HueAdjustments::NAMES[i]);
                });
                changed |= slider(ui, "Farbton", &mut band.hue, -30.0..=30.0, 0.0);
                changed |= slider(ui, "Sättigung", &mut band.saturation, -1.0..=1.0, 0.0);
                changed |= slider(ui, "Helligkeit", &mut band.lightness, -1.0..=1.0, 0.0);
                ui.add_space(4.0);
            }
        });
    }

    section(ui, "Split-Toning", false, |ui| {
        changed |= tone(ui, "Schatten", &mut r.shadow_tone);
        changed |= tone(ui, "Lichter", &mut r.highlight_tone);
    });

    section(ui, "Korn & Vignette", true, |ui| {
        changed |= slider(ui, "Korn", &mut r.grain, 0.0..=1.0, 0.0);
        changed |= slider(ui, "Korngröße", &mut r.grain_size, 0.75..=4.0, 1.5);
        changed |= slider(ui, "Vignette", &mut r.vignette, -1.0..=1.0, 0.0);
    });

    changed
}

fn section(ui: &mut Ui, title: &str, open: bool, add: impl FnOnce(&mut Ui)) {
    CollapsingHeader::new(RichText::new(title).strong())
        .default_open(open)
        .show(ui, add);
}

fn slider(ui: &mut Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, default: f32) -> bool {
    let resp = ui.add(Slider::new(value, range).text(label).fixed_decimals(2));
    if resp.double_clicked() {
        *value = default;
        return true;
    }
    resp.changed()
}

fn tone(ui: &mut Ui, label: &str, tone: &mut Tone) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let color = oklch_color(0.72, 0.03 + 0.12 * tone.strength.min(1.0), tone.hue);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 6.0, color);
        ui.strong(label);
    });
    changed |= slider(ui, "Farbton °", &mut tone.hue, 0.0..=360.0, 0.0);
    changed |= slider(ui, "Stärke", &mut tone.strength, 0.0..=1.0, 0.0);
    changed
}

fn oklch_color(l: f32, c: f32, hue_deg: f32) -> Color32 {
    let (s, co) = hue_deg.to_radians().sin_cos();
    let rgb = oklab_to_linear_srgb([l, c * co, c * s]).map(|v| (linear_to_srgb(v.clamp(0.0, 1.0)) * 255.0) as u8);
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}
