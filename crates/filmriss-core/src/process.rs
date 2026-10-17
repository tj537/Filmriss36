//! CPU-Referenzpipeline. Muss sich exakt so verhalten wie `preview.wgsl` in der App,
//! damit das gespeicherte Foto genauso aussieht wie die Live-Preview.
//!
//! Reihenfolge: LUT → Mischung mit Original (Intensität) → Vignette → Korn.

use rayon::prelude::*;

use crate::color::smoothstep;
use crate::lut::Lut3D;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LookParams {
    /// 0 = Original, 1 = volle Simulation.
    pub intensity: f32,
    pub grain: f32,
    pub grain_size: f32,
    pub vignette: f32,
    pub seed: u32,
}

impl Default for LookParams {
    fn default() -> Self {
        Self {
            intensity: 1.0,
            grain: 0.0,
            grain_size: 1.5,
            vignette: 0.0,
            seed: 0,
        }
    }
}

/// Integer-Hash (lowbias32-Variante). Identisch im WGSL-Shader implementiert.
#[inline]
pub fn hash(x: u32, y: u32, seed: u32) -> u32 {
    let mut h = x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    h
}

#[inline]
fn hash_signed(x: u32, y: u32, seed: u32) -> f32 {
    (hash(x, y, seed) >> 8) as f32 * (2.0 / 16_777_216.0) - 1.0
}

/// Weiches Wert-Rauschen (−1 … 1) – wirkt organischer als reines Pixelrauschen.
/// `px`/`py` sind Bildkoordinaten in Pixeln (Pixelmitte = Index + 0.5).
#[inline]
pub fn grain_noise(px: f32, py: f32, size: f32, seed: u32) -> f32 {
    let gx = px / size.max(0.5);
    let gy = py / size.max(0.5);
    let (ix, iy) = (gx.floor(), gy.floor());
    let (fx, fy) = (gx - ix, gy - iy);
    let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (x0, y0) = (ix as u32, iy as u32);
    let n00 = hash_signed(x0, y0, seed);
    let n10 = hash_signed(x0.wrapping_add(1), y0, seed);
    let n01 = hash_signed(x0, y0.wrapping_add(1), seed);
    let n11 = hash_signed(x0.wrapping_add(1), y0.wrapping_add(1), seed);
    let a = n00 + (n10 - n00) * ux;
    let b = n01 + (n11 - n01) * ux;
    (a + (b - a) * uy) * 1.6
}

/// Bearbeitet einen einzelnen Pixel (sRGB 0..1).
#[inline]
pub fn shade(src: [f32; 3], lut: &Lut3D, px: f32, py: f32, width: f32, height: f32, p: &LookParams) -> [f32; 3] {
    let graded = lut.sample(src);
    let mut c = [0.0; 3];
    for i in 0..3 {
        c[i] = src[i] + (graded[i] - src[i]) * p.intensity;
    }

    if p.vignette != 0.0 {
        let aspect = width / height;
        let u = (px / width - 0.5) * aspect;
        let v = py / height - 0.5;
        let d = (u * u + v * v).sqrt() / ((aspect * 0.5).powi(2) + 0.25).sqrt();
        let f = 1.0 - p.vignette * 0.6 * smoothstep(0.35, 1.05, d);
        c = c.map(|x| x * f);
    }

    if p.grain > 0.0 {
        let l = (0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]).clamp(0.0, 1.0);
        // Korn ist in den Mitteltönen am sichtbarsten, in Tiefen/Lichtern schwächer.
        let amp = p.grain * 0.14 * (0.25 + 3.0 * l * (1.0 - l));
        let n = grain_noise(px, py, p.grain_size, p.seed) * amp;
        c = c.map(|x| x + n);
    }

    c.map(|x| x.clamp(0.0, 1.0))
}

/// Wendet den Look in-place auf ein RGBA8-Bild an (parallel über Zeilen).
pub fn render_rgba8(pixels: &mut [u8], width: u32, height: u32, lut: &Lut3D, p: &LookParams) {
    let (w, h) = (width as f32, height as f32);
    pixels
        .par_chunks_mut(width as usize * 4)
        .enumerate()
        .for_each(|(y, row)| {
            let py = y as f32 + 0.5;
            for (x, px) in row.chunks_exact_mut(4).enumerate() {
                let src = [px[0] as f32 / 255.0, px[1] as f32 / 255.0, px[2] as f32 / 255.0];
                let out = shade(src, lut, x as f32 + 0.5, py, w, h, p);
                px[0] = (out[0] * 255.0 + 0.5) as u8;
                px[1] = (out[1] * 255.0 + 0.5) as u8;
                px[2] = (out[2] * 255.0 + 0.5) as u8;
            }
        });
}

/// Schnelles Verkleinern (2×2-Mittelwert pro Zielpixel) für Vorschaubilder.
pub fn downscale_rgba8(src: &[u8], width: u32, height: u32, tw: u32, th: u32) -> Vec<u8> {
    let mut out = vec![0u8; (tw * th * 4) as usize];
    let (w, h) = (width as usize, height as usize);
    for ty in 0..th as usize {
        for tx in 0..tw as usize {
            let mut acc = [0u32; 4];
            for (oy, ox) in [(0.25, 0.25), (0.25, 0.75), (0.75, 0.25), (0.75, 0.75)] {
                let sx = (((tx as f32 + ox) / tw as f32) * w as f32) as usize;
                let sy = (((ty as f32 + oy) / th as f32) * h as f32) as usize;
                let i = (sy.min(h - 1) * w + sx.min(w - 1)) * 4;
                for c in 0..4 {
                    acc[c] += src[i + c] as u32;
                }
            }
            let o = (ty * tw as usize + tx) * 4;
            for c in 0..4 {
                out[o + c] = (acc[c] / 4) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_look_is_lossless() {
        let lut = Lut3D::identity(33);
        let mut img: Vec<u8> = (0..64 * 4).map(|i| (i * 7 % 256) as u8).collect();
        let orig = img.clone();
        render_rgba8(&mut img, 8, 8, &lut, &LookParams::default());
        assert_eq!(img, orig);
    }

    #[test]
    fn grain_noise_is_bounded_and_varies() {
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        for y in 0..64 {
            for x in 0..64 {
                let n = grain_noise(x as f32 + 0.5, y as f32 + 0.5, 1.5, 42);
                min = min.min(n);
                max = max.max(n);
            }
        }
        assert!(min >= -1.7 && max <= 1.7 && max - min > 1.0);
    }
}
