//! Film-Rezepte: eine Filmsimulation als Satz verständlicher Parameter.
//!
//! Ein Rezept wird nie pro Pixel ausgewertet, sondern einmalig in eine 3D-LUT
//! "gebacken" ([`FilmRecipe::bake`]). Dadurch darf die Mathematik hier beliebig
//! aufwendig sein (Oklab, Gamut-Mapping, Farbton-Kurven …) – die Live-Preview
//! kostet trotzdem nur einen Texturzugriff pro Pixel.
//!
//! Rezepte werden als TOML gespeichert, fehlende Felder fallen auf neutrale Werte
//! zurück. Ein minimales Rezept ist also z. B.:
//!
//! ```toml
//! name = "Mein Look"
//! contrast = 0.2
//! saturation = -0.1
//! ```

use serde::{Deserialize, Serialize};
use std::f32::consts::PI;

use crate::color::{
    linear_srgb_to_oklab, linear_to_srgb3, luminance, mix, oklab_to_linear_srgb, smoothstep, srgb_to_linear3,
};
use crate::lut::Lut3D;

/// Oklab-Helligkeit eines 18%-Graus – Drehpunkt der Kontrastkurve.
const MID_GREY_L: f32 = 0.565;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilmRecipe {
    pub name: String,
    pub description: String,

    /// Optional: Dateiname einer `.cube`-LUT (im LUT-Ordner), die *vor* dem Rezept
    /// angewendet wird. So lässt sich jede fremde LUT mit Reglern feintunen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_lut: Option<String>,

    /// Belichtungskorrektur in Blendenstufen (−2 … +2).
    pub exposure: f32,
    /// Weißabgleich-Verschiebung: + wärmer (gelb/rot), − kühler (blau). Bereich −1 … 1.
    pub temperature: f32,
    /// + Richtung Magenta, − Richtung Grün. Bereich −1 … 1.
    pub tint: f32,

    /// S-Kurve: + mehr, − weniger Kontrast. Bereich −1 … 1.
    pub contrast: f32,
    /// Lichter heller (+) oder weicher/dunkler (−). Bereich −1 … 1.
    pub highlights: f32,
    /// Schatten heller (+) oder tiefer (−). Bereich −1 … 1.
    pub shadows: f32,
    /// Angehobenes Schwarz ("Fade", ausgeblichener Abzug). Bereich 0 … 1.
    pub fade: f32,

    /// Globale Sättigung. −1 = Graustufen, +1 = doppelte Chroma.
    pub saturation: f32,
    /// Farbton-/Sättigungs-/Helligkeitskorrektur pro Farbbereich.
    pub hues: HueAdjustments,

    /// Schwarzweiß-Modus.
    pub monochrome: bool,
    /// Kanalmischung für Schwarzweiß (linear RGB). Rotfilter = viel R, wenig B usw.
    pub mono_mix: [f32; 3],

    /// Tönung der Schatten (z. B. kühles Cyan bei Negativfilmen).
    pub shadow_tone: Tone,
    /// Tönung der Lichter (z. B. warmes Bernstein).
    pub highlight_tone: Tone,

    /// Filmkorn-Stärke 0 … 1 (wird live im Shader erzeugt, nicht in der LUT).
    pub grain: f32,
    /// Korngröße in Kamerapixeln (1 = fein, 3 = grob).
    pub grain_size: f32,
    /// Vignette: + Ecken dunkler, − Ecken heller. Bereich −1 … 1 (Shader).
    pub vignette: f32,
}

impl Default for FilmRecipe {
    fn default() -> Self {
        Self {
            name: "Ohne Namen".into(),
            description: String::new(),
            base_lut: None,
            exposure: 0.0,
            temperature: 0.0,
            tint: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            fade: 0.0,
            saturation: 0.0,
            hues: HueAdjustments::default(),
            monochrome: false,
            mono_mix: [0.2126, 0.7152, 0.0722],
            shadow_tone: Tone::default(),
            highlight_tone: Tone::default(),
            grain: 0.0,
            grain_size: 1.5,
            vignette: 0.0,
        }
    }
}

/// Farbtönung über einen Farbtonwinkel (Grad, Oklch) und eine Stärke 0 … 1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tone {
    pub hue: f32,
    pub strength: f32,
}

/// Anpassung eines Farbbereichs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HueBand {
    /// Farbtonverschiebung in Grad (−30 … 30 ist sinnvoll).
    pub hue: f32,
    /// Sättigung −1 … 1.
    pub saturation: f32,
    /// Helligkeit −1 … 1.
    pub lightness: f32,
}

impl HueBand {
    fn is_neutral(&self) -> bool {
        self.hue == 0.0 && self.saturation == 0.0 && self.lightness == 0.0
    }
}

/// Sieben Farbbereiche wie in Lightroom/Capture One. Die Zentren liegen auf den
/// Oklch-Farbtönen der sRGB-Primär-/Sekundärfarben (+ Orange für Hauttöne).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HueAdjustments {
    pub red: HueBand,
    pub orange: HueBand,
    pub yellow: HueBand,
    pub green: HueBand,
    pub cyan: HueBand,
    pub blue: HueBand,
    pub magenta: HueBand,
}

impl HueAdjustments {
    pub const CENTERS: [f32; 7] = [29.0, 55.0, 110.0, 142.0, 195.0, 264.0, 328.0];
    pub const NAMES: [&'static str; 7] = ["Rot", "Orange", "Gelb", "Grün", "Cyan", "Blau", "Magenta"];

    pub fn bands(&self) -> [&HueBand; 7] {
        [
            &self.red,
            &self.orange,
            &self.yellow,
            &self.green,
            &self.cyan,
            &self.blue,
            &self.magenta,
        ]
    }

    pub fn bands_mut(&mut self) -> [&mut HueBand; 7] {
        [
            &mut self.red,
            &mut self.orange,
            &mut self.yellow,
            &mut self.green,
            &mut self.cyan,
            &mut self.blue,
            &mut self.magenta,
        ]
    }

    fn is_neutral(&self) -> bool {
        self.bands().iter().all(|b| b.is_neutral())
    }

    /// Interpoliert die Bandwerte für einen Farbton (Grad) – weiche Übergänge,
    /// die Gewichte zweier Nachbarbänder summieren sich immer zu 1.
    fn at(&self, hue_deg: f32) -> HueBand {
        let bands = self.bands();
        let h = hue_deg.rem_euclid(360.0);
        let n = Self::CENTERS.len();
        for i in 0..n {
            let c0 = Self::CENTERS[i];
            let c1 = if i + 1 < n {
                Self::CENTERS[i + 1]
            } else {
                Self::CENTERS[0] + 360.0
            };
            let hh = if h < c0 { h + 360.0 } else { h };
            if hh >= c0 && hh < c1 {
                let t = smoothstep(0.0, 1.0, (hh - c0) / (c1 - c0));
                let (a, b) = (bands[i], bands[(i + 1) % n]);
                return HueBand {
                    hue: mix(a.hue, b.hue, t),
                    saturation: mix(a.saturation, b.saturation, t),
                    lightness: mix(a.lightness, b.lightness, t),
                };
            }
        }
        *bands[0]
    }
}

impl FilmRecipe {
    pub fn neutral(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("Rezept ist immer serialisierbar")
    }

    /// Backt das Rezept (optional nach einer Basis-LUT) in eine 3D-LUT.
    pub fn bake(&self, size: usize, base: Option<&Lut3D>) -> Lut3D {
        match base {
            Some(base) => Lut3D::from_fn(size, |c| self.apply(base.sample(c))),
            None => Lut3D::from_fn(size, |c| self.apply(c)),
        }
    }

    /// Wendet das Rezept auf eine sRGB-kodierte Farbe (0..1) an.
    pub fn apply(&self, srgb: [f32; 3]) -> [f32; 3] {
        let mut lin = srgb_to_linear3(srgb);

        // 1) Belichtung + Weißabgleich (linear, wie in der Kamera)
        let ev = 2f32.powf(self.exposure);
        let gains = [
            (0.18 * self.temperature).exp(),
            (-0.12 * self.tint).exp(),
            (-0.18 * self.temperature).exp(),
        ];
        let norm = 1.0 / luminance(gains);
        for i in 0..3 {
            lin[i] *= ev * gains[i] * norm;
        }

        // 2) Schwarzweiß: Kanalmischer (Farbfilter-Effekt)
        if self.monochrome {
            let w = self.mono_mix;
            let sum = (w[0] + w[1] + w[2]).max(1e-4);
            let y = ((lin[0] * w[0] + lin[1] * w[1] + lin[2] * w[2]) / sum).max(0.0);
            lin = [y, y, y];
        }

        // 3) Alles Weitere in Oklch (Helligkeit, Chroma, Farbton getrennt)
        let [l, a, b] = linear_srgb_to_oklab(lin);
        let mut l = l.max(0.0);
        let mut chroma = (a * a + b * b).sqrt();
        let mut hue = b.atan2(a).to_degrees();

        if !self.monochrome && !self.hues.is_neutral() {
            let band = self.hues.at(hue);
            // Nahezu neutrale Farben sollen nicht eingefärbt/verschoben werden.
            let colorful = smoothstep(0.0, 0.06, chroma);
            hue += band.hue * colorful;
            chroma *= (1.0 + band.saturation).max(0.0);
            l += band.lightness * 0.12 * colorful;
        }
        chroma *= (1.0 + self.saturation).max(0.0);

        // 4) Tonkurve auf der Helligkeit; Chroma wandert ein Stück mit (wie bei Film)
        let l_new = self.tone_curve(l);
        if l > 1e-4 {
            chroma *= (l_new / l).clamp(0.25, 4.0).sqrt();
        }
        l = l_new;

        let (sin, cos) = hue.to_radians().sin_cos();
        let mut a = chroma * cos;
        let mut b = chroma * sin;

        // 5) Split-Toning
        let ws = 1.0 - smoothstep(0.0, MID_GREY_L + 0.1, l);
        let wh = smoothstep(MID_GREY_L - 0.1, 1.0, l);
        for (tone, w) in [(self.shadow_tone, ws), (self.highlight_tone, wh)] {
            if tone.strength != 0.0 {
                let (s, c) = (tone.hue * PI / 180.0).sin_cos();
                a += c * tone.strength * 0.06 * w;
                b += s * tone.strength * 0.06 * w;
            }
        }

        // 6) zurück nach sRGB, mit weichem Gamut-Mapping statt hartem Clipping
        let lin = gamut_map([l, a, b]);
        let out = linear_to_srgb3(lin);
        out.map(|v| v.clamp(0.0, 1.0))
    }

    fn tone_curve(&self, x: f32) -> f32 {
        let p = MID_GREY_L;
        let mut x = x.clamp(0.0, 1.0);

        // Kontrast: Potenzkurve um den Mittelgrau-Punkt (stetig differenzierbar)
        if self.contrast != 0.0 {
            let e = 2f32.powf(self.contrast);
            x = if x < p {
                p * (x / p).powf(e)
            } else {
                1.0 - (1.0 - p) * ((1.0 - x) / (1.0 - p)).powf(e)
            };
        }
        if self.highlights != 0.0 {
            let w = smoothstep(p - 0.15, 1.0, x);
            x += self.highlights * 0.25 * w * (1.0 - x);
        }
        if self.shadows != 0.0 {
            let w = 1.0 - smoothstep(0.0, p + 0.15, x);
            x += self.shadows * 0.25 * w * x;
        }
        if self.fade > 0.0 {
            let lift = self.fade * 0.2;
            x = lift + x * (1.0 - lift);
        }
        x.clamp(0.0, 1.0)
    }
}

/// Reduziert die Chroma, bis die Farbe in den sRGB-Farbraum passt (Farbton und
/// Helligkeit bleiben erhalten). Liefert lineares sRGB.
fn gamut_map([l, a, b]: [f32; 3]) -> [f32; 3] {
    let in_gamut = |c: [f32; 3]| c.iter().all(|v| (-1e-4..=1.0 + 1e-4).contains(v));
    let rgb = oklab_to_linear_srgb([l, a, b]);
    if in_gamut(rgb) || l <= 0.0 || l >= 1.0 {
        return rgb.map(|v| v.clamp(0.0, 1.0));
    }
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..12 {
        let mid = 0.5 * (lo + hi);
        if in_gamut(oklab_to_linear_srgb([l, a * mid, b * mid])) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    oklab_to_linear_srgb([l, a * lo, b * lo]).map(|v| v.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3], eps: f32) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < eps)
    }

    #[test]
    fn default_recipe_is_identity() {
        let r = FilmRecipe::default();
        for c in [
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [0.5, 0.2, 0.9],
            [0.9, 0.7, 0.1],
            [0.05, 0.4, 0.3],
        ] {
            let out = r.apply(c);
            assert!(close(out, c, 2e-3), "{c:?} -> {out:?}");
        }
    }

    #[test]
    fn monochrome_is_grey() {
        let r = FilmRecipe {
            monochrome: true,
            ..Default::default()
        };
        let out = r.apply([0.8, 0.3, 0.1]);
        assert!((out[0] - out[1]).abs() < 2e-3 && (out[1] - out[2]).abs() < 2e-3);
    }

    #[test]
    fn contrast_keeps_endpoints() {
        let r = FilmRecipe {
            contrast: 0.8,
            ..Default::default()
        };
        assert!(close(r.apply([0.0; 3]), [0.0; 3], 1e-3));
        assert!(close(r.apply([1.0; 3]), [1.0; 3], 1e-3));
    }

    #[test]
    fn tone_curve_is_monotonic_for_extremes() {
        for contrast in [-1.0, 0.0, 1.0] {
            for hl in [-1.0, 1.0] {
                for sh in [-1.0, 1.0] {
                    let r = FilmRecipe {
                        contrast,
                        highlights: hl,
                        shadows: sh,
                        fade: 0.5,
                        ..Default::default()
                    };
                    let mut prev = -1.0;
                    for i in 0..=200 {
                        let y = r.tone_curve(i as f32 / 200.0);
                        assert!(y >= prev - 1e-5, "nicht monoton bei {contrast} {hl} {sh}");
                        prev = y;
                    }
                }
            }
        }
    }

    #[test]
    fn toml_roundtrip_and_defaults() {
        let r: FilmRecipe = FilmRecipe::from_toml("name = \"x\"\ncontrast = 0.3\n[hues.blue]\nhue = -10\n").unwrap();
        assert_eq!(r.contrast, 0.3);
        assert_eq!(r.hues.blue.hue, -10.0);
        assert_eq!(r.saturation, 0.0);
        let back = FilmRecipe::from_toml(&r.to_toml()).unwrap();
        assert_eq!(back, r);
    }
}
