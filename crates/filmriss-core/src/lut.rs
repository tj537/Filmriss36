//! 3D LUT (lookup table) – the heart of every film simulation.
//!
//! A 3D LUT maps every input color (R,G,B) to an output color. It stores a grid of
//! `size³` sample points and interpolates trilinearly in between. On the GPU the
//! texture hardware does this (3D texture + linear filter) – so an arbitrarily complex
//! look costs exactly one texture fetch per pixel.
//!
//! The data layout matches the `.cube` format: R varies fastest, then G, then B.

use rayon::prelude::*;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LutError {
    #[error("line {line}: {msg}")]
    Parse { line: usize, msg: String },
    #[error("LUT_3D_SIZE is missing")]
    MissingSize,
    #[error("1D LUTs are not supported (yet)")]
    Unsupported1D,
    #[error("LUT size {0} is invalid (allowed: 2..=128)")]
    InvalidSize(usize),
    #[error("expected {expected} entries, found {found}")]
    WrongEntryCount { expected: usize, found: usize },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lut3D {
    size: usize,
    data: Vec<[f32; 3]>,
}

impl Lut3D {
    /// Builds a LUT by evaluating `f` at every grid position (in parallel).
    /// Input and output are sRGB-encoded values in the range 0..1.
    pub fn from_fn(size: usize, f: impl Fn([f32; 3]) -> [f32; 3] + Sync) -> Self {
        assert!((2..=128).contains(&size), "invalid LUT size {size}");
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

    /// Raw data in `.cube` layout (R fastest). Fits directly into a 3D texture
    /// with x = R, y = G, z = B.
    pub fn data(&self) -> &[[f32; 3]] {
        &self.data
    }

    #[inline]
    fn at(&self, r: usize, g: usize, b: usize) -> [f32; 3] {
        self.data[r + self.size * (g + self.size * b)]
    }

    /// Trilinear interpolation – identical to what the GPU texture unit does.
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

    /// Parses an Adobe/Resolve `.cube` file (3D). `DOMAIN_MIN/MAX` and
    /// `LUT_3D_INPUT_RANGE` are supported and normalized to 0..1.
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
                    .map(|p| p.parse::<f32>().map_err(|_| err("number expected")))
                    .collect()
            };

            match first {
                "TITLE" => {}
                "LUT_1D_SIZE" => return Err(LutError::Unsupported1D),
                "LUT_3D_SIZE" => {
                    let n: usize = parts
                        .next()
                        .and_then(|s| s.parse().ok())
                        .ok_or_else(|| err("size expected"))?;
                    if !(2..=128).contains(&n) {
                        return Err(LutError::InvalidSize(n));
                    }
                    size = Some(n);
                    data.reserve(n * n * n);
                }
                "DOMAIN_MIN" | "DOMAIN_MAX" => {
                    let v = floats(parts)?;
                    let v: [f32; 3] = v.try_into().map_err(|_| err("3 values expected"))?;
                    if first == "DOMAIN_MIN" {
                        domain_min = v
                    } else {
                        domain_max = v
                    }
                }
                "LUT_3D_INPUT_RANGE" => {
                    let v = floats(parts)?;
                    let [lo, hi]: [f32; 2] = v.try_into().map_err(|_| err("2 values expected"))?;
                    domain_min = [lo; 3];
                    domain_max = [hi; 3];
                }
                kw if kw.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) => {
                    log::debug!("ignoring unknown .cube keyword: {kw}");
                }
                _ => {
                    let v = floats(line.split_whitespace())?;
                    let v: [f32; 3] = v.try_into().map_err(|_| err("3 values expected"))?;
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
        // Non-standard domain: remap to 0..1 so GPU and CPU only have to handle one case.
        Ok(Self::from_fn(size, |c| {
            let mut x = [0.0; 3];
            for i in 0..3 {
                x[i] = (c[i] - domain_min[i]) / (domain_max[i] - domain_min[i]);
            }
            lut.sample(x)
        }))
    }

    /// Exports as `.cube` – so custom recipes can also be used in
    /// Lightroom, Photoshop, Resolve, Final Cut etc.
    pub fn to_cube_string(&self, title: &str) -> String {
        use std::fmt::Write as _;
        let mut out = String::with_capacity(self.data.len() * 28 + 128);
        let _ = writeln!(out, "# Created with Filmriss");
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
        let mut text = String::from("# comment\nTITLE \"x\"\nLUT_3D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 2 2 2\n");
        // Identity over domain 0..2: input 0..1 lands in the lower half of the grid.
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
