//! 3D-LUT (Lookup Table) – das Herzstück jeder Filmsimulation.
//!
//! Eine 3D-LUT bildet jede Eingangsfarbe (R,G,B) auf eine Ausgangsfarbe ab. Gespeichert
//! wird ein Gitter aus `size³` Stützstellen; dazwischen wird trilinear interpoliert.
//! Auf der GPU übernimmt das die Textur-Hardware (3D-Textur + linearer Filter) – damit
//! kostet ein beliebig komplexer Look pro Pixel genau einen Texturzugriff.
//!
//! Datenlayout entspricht dem `.cube`-Format: R läuft am schnellsten, dann G, dann B.

use rayon::prelude::*;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LutError {
    #[error("Zeile {line}: {msg}")]
    Parse { line: usize, msg: String },
    #[error("LUT_3D_SIZE fehlt")]
    MissingSize,
    #[error("1D-LUTs werden (noch) nicht unterstützt")]
    Unsupported1D,
    #[error("LUT-Größe {0} ist ungültig (erlaubt: 2..=128)")]
    InvalidSize(usize),
    #[error("erwartet {expected} Einträge, gefunden {found}")]
    WrongEntryCount { expected: usize, found: usize },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lut3D {
    size: usize,
    data: Vec<[f32; 3]>,
}

impl Lut3D {
    /// Erzeugt eine LUT, indem `f` für jede Gitterposition ausgewertet wird (parallel).
    /// Ein- und Ausgabe sind sRGB-kodierte Werte im Bereich 0..1.
    pub fn from_fn(size: usize, f: impl Fn([f32; 3]) -> [f32; 3] + Sync) -> Self {
        assert!((2..=128).contains(&size), "ungültige LUT-Größe {size}");
        let scale = 1.0 / (size - 1) as f32;
        let data = (0..size * size * size)
            .into_par_iter()
            .map(|i| {
                let r = i % size;
                let g = (i / size) % size;
                let b = i / (size * size);
                f([r as f32 * scale, g as f32 * scale, b as f32 * scale])
            })
            .collect();
        Self { size, data }
    }

    pub fn identity(size: usize) -> Self {
        Self::from_fn(size, |c| c)
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// Rohdaten im `.cube`-Layout (R am schnellsten). Passt direkt in eine 3D-Textur
    /// mit x = R, y = G, z = B.
    pub fn data(&self) -> &[[f32; 3]] {
        &self.data
    }

    #[inline]
    fn at(&self, r: usize, g: usize, b: usize) -> [f32; 3] {
        self.data[r + self.size * (g + self.size * b)]
    }

    /// Trilineare Interpolation – identisch zu dem, was die GPU-Textureinheit macht.
    #[inline]
    pub fn sample(&self, rgb: [f32; 3]) -> [f32; 3] {
        let max = (self.size - 1) as f32;
        let mut idx = [0usize; 3];
        let mut frac = [0f32; 3];
        for i in 0..3 {
            let x = rgb[i].clamp(0.0, 1.0) * max;
            let i0 = (x as usize).min(self.size - 2);
            idx[i] = i0;
            frac[i] = x - i0 as f32;
        }
        let [r, g, b] = idx;
        let [fr, fg, fb] = frac;

        let lerp3 = |a: [f32; 3], c: [f32; 3], t: f32| {
            [
                a[0] + (c[0] - a[0]) * t,
                a[1] + (c[1] - a[1]) * t,
                a[2] + (c[2] - a[2]) * t,
            ]
        };
        let c00 = lerp3(self.at(r, g, b), self.at(r + 1, g, b), fr);
        let c10 = lerp3(self.at(r, g + 1, b), self.at(r + 1, g + 1, b), fr);
        let c01 = lerp3(self.at(r, g, b + 1), self.at(r + 1, g, b + 1), fr);
        let c11 = lerp3(self.at(r, g + 1, b + 1), self.at(r + 1, g + 1, b + 1), fr);
        let c0 = lerp3(c00, c10, fg);
        let c1 = lerp3(c01, c11, fg);
        lerp3(c0, c1, fb)
    }

    /// Parst eine Adobe/Resolve `.cube`-Datei (3D). `DOMAIN_MIN/MAX` und
    /// `LUT_3D_INPUT_RANGE` werden unterstützt und auf 0..1 normalisiert.
    pub fn parse_cube(text: &str) -> Result<Self, LutError> {
        let mut size = None;
        let mut domain_min = [0.0f32; 3];
        let mut domain_max = [1.0f32; 3];
        let mut data: Vec<[f32; 3]> = Vec::new();

        for (lineno, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let err = |msg: &str| LutError::Parse {
                line: lineno + 1,
                msg: msg.to_owned(),
            };
            let mut parts = line.split_whitespace();
            let first = parts.next().unwrap_or_default();

            let floats = |parts: std::str::SplitWhitespace<'_>| -> Result<Vec<f32>, LutError> {
                parts
                    .map(|p| p.parse::<f32>().map_err(|_| err("Zahl erwartet")))
                    .collect()
            };

            match first {
                "TITLE" => {}
                "LUT_1D_SIZE" => return Err(LutError::Unsupported1D),
                "LUT_3D_SIZE" => {
                    let n: usize = parts
                        .next()
                        .and_then(|s| s.parse().ok())
                        .ok_or_else(|| err("Größe erwartet"))?;
                    if !(2..=128).contains(&n) {
                        return Err(LutError::InvalidSize(n));
                    }
                    size = Some(n);
                    data.reserve(n * n * n);
                }
                "DOMAIN_MIN" | "DOMAIN_MAX" => {
                    let v = floats(parts)?;
                    let v: [f32; 3] = v.try_into().map_err(|_| err("3 Werte erwartet"))?;
                    if first == "DOMAIN_MIN" {
                        domain_min = v
                    } else {
                        domain_max = v
                    }
                }
                "LUT_3D_INPUT_RANGE" => {
                    let v = floats(parts)?;
                    let [lo, hi]: [f32; 2] = v.try_into().map_err(|_| err("2 Werte erwartet"))?;
                    domain_min = [lo; 3];
                    domain_max = [hi; 3];
                }
                kw if kw.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) => {
                    log::debug!("unbekanntes .cube-Schlüsselwort ignoriert: {kw}");
                }
                _ => {
                    let v = floats(line.split_whitespace())?;
                    let v: [f32; 3] = v.try_into().map_err(|_| err("3 Werte erwartet"))?;
                    data.push(v);
                }
            }
        }

        let size = size.ok_or(LutError::MissingSize)?;
        let expected = size * size * size;
        if data.len() != expected {
            return Err(LutError::WrongEntryCount {
                expected,
                found: data.len(),
            });
        }
        let lut = Self { size, data };

        if domain_min == [0.0; 3] && domain_max == [1.0; 3] {
            return Ok(lut);
        }
        // Nicht-standard Domain: auf 0..1 umrechnen, damit GPU und CPU nur einen Fall kennen.
        Ok(Self::from_fn(size, |c| {
            let mut x = [0.0; 3];
            for i in 0..3 {
                x[i] = (c[i] - domain_min[i]) / (domain_max[i] - domain_min[i]);
            }
            lut.sample(x)
        }))
    }

    /// Exportiert als `.cube` – damit lassen sich eigene Rezepte auch in
    /// Lightroom, Photoshop, Resolve, Final Cut usw. verwenden.
    pub fn to_cube_string(&self, title: &str) -> String {
        use std::fmt::Write as _;
        let mut out = String::with_capacity(self.data.len() * 28 + 128);
        let _ = writeln!(out, "# Erstellt mit Filmriss");
        let _ = writeln!(out, "TITLE \"{}\"", title.replace('"', "'"));
        let _ = writeln!(out, "LUT_3D_SIZE {}", self.size);
        let _ = writeln!(out, "DOMAIN_MIN 0.0 0.0 0.0");
        let _ = writeln!(out, "DOMAIN_MAX 1.0 1.0 1.0");
        for [r, g, b] in &self.data {
            let _ = writeln!(out, "{r:.6} {g:.6} {b:.6}");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_samples_exactly() {
        let lut = Lut3D::identity(17);
        for c in [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.3, 0.6, 0.123]] {
            let s = lut.sample(c);
            for i in 0..3 {
                assert!((s[i] - c[i]).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn cube_roundtrip() {
        let lut = Lut3D::from_fn(5, |[r, g, b]| [g, b, r * 0.5]);
        let text = lut.to_cube_string("test");
        let parsed = Lut3D::parse_cube(&text).unwrap();
        assert_eq!(parsed.size(), 5);
        for (a, b) in lut.data().iter().zip(parsed.data()) {
            for i in 0..3 {
                assert!((a[i] - b[i]).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn cube_rejects_wrong_count() {
        let text = "LUT_3D_SIZE 2\n0 0 0\n1 1 1\n";
        assert!(matches!(Lut3D::parse_cube(text), Err(LutError::WrongEntryCount { .. })));
    }

    #[test]
    fn cube_handles_comments_and_domain() {
        let mut text = String::from("# Kommentar\nTITLE \"x\"\nLUT_3D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 2 2 2\n");
        // Identität über Domain 0..2: Eingabe 0..1 landet in der unteren Gitterhälfte.
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    text.push_str(&format!("{} {} {}\n", r * 2, g * 2, b * 2));
                }
            }
        }
        let lut = Lut3D::parse_cube(&text).unwrap();
        let s = lut.sample([0.25, 0.5, 1.0]);
        assert!((s[0] - 0.25).abs() < 1e-5 && (s[1] - 0.5).abs() < 1e-5 && (s[2] - 1.0).abs() < 1e-5);
    }
}
