//! Animated test pattern: ColorChecker patches, color wheel, gray and hue ramps and a
//! moving "sun". Lets you judge looks without a camera, and you immediately see
//! whether the preview stutters.

use std::time::{Duration, Instant};

use crate::stream::{Shared, StreamState};

const W: u32 = 1280;
const H: u32 = 720;
const FPS: u32 = 30;

/// Approximate sRGB values of the X-Rite ColorChecker Classic (24 patches).
const CHECKER: [[u8; 3]; 24] = [
    [115, 82, 68],
    [194, 150, 130],
    [98, 122, 157],
    [87, 108, 67],
    [133, 128, 177],
    [103, 189, 170],
    [214, 126, 44],
    [80, 91, 166],
    [193, 90, 99],
    [94, 60, 108],
    [157, 188, 64],
    [224, 163, 46],
    [56, 61, 150],
    [70, 148, 73],
    [175, 54, 60],
    [231, 199, 31],
    [187, 86, 149],
    [8, 133, 161],
    [243, 243, 242],
    [200, 200, 200],
    [160, 160, 160],
    [122, 122, 121],
    [85, 85, 85],
    [52, 52, 52],
];

pub(crate) fn run(shared: &Shared) {
    let background = render_background();
    shared.set_state(StreamState::Running {
        width: W,
        height: H,
        format: "test pattern".into(),
    });

    let start = Instant::now();
    let frame_time = Duration::from_secs_f64(1.0 / FPS as f64);
    let mut next = Instant::now();

    while !shared.should_stop() {
        let mut buf = shared.buffer(background.len());
        buf.copy_from_slice(&background);
        draw_sun(&mut buf, start.elapsed().as_secs_f32());
        shared.publish(W, H, buf);

        next += frame_time;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            next = now;
        }
    }
}

fn put(buf: &mut [u8], x: u32, y: u32, c: [u8; 3]) {
    let i = ((y * W + x) * 4) as usize;
    buf[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
}

fn hsv(h: f32, s: f32, v: f32) -> [u8; 3] {
    let h6 = (h.rem_euclid(1.0)) * 6.0;
    let f = |n: f32| {
        let k = (n + h6) % 6.0;
        v - v * s * (k.min(4.0 - k).clamp(0.0, 1.0))
    };
    [(f(5.0) * 255.0) as u8, (f(3.0) * 255.0) as u8, (f(1.0) * 255.0) as u8]
}

fn render_background() -> Vec<u8> {
    let mut buf = vec![0u8; (W * H * 4) as usize];

    // Background: "sky" at the top → warm "ground" at the bottom
    for y in 0..H {
        let t = y as f32 / H as f32;
        let top = [70.0, 110.0, 160.0];
        let bottom = [150.0, 120.0, 90.0];
        let c = [0, 1, 2].map(|i| (top[i] + (bottom[i] - top[i]) * t) as u8);
        for x in 0..W {
            put(&mut buf, x, y, c);
        }
    }

    // ColorChecker 6×4
    let (patch, gap, ox, oy) = (96, 12, 40, 40);
    for (i, c) in CHECKER.iter().enumerate() {
        let (col, row) = (i as u32 % 6, i as u32 / 6);
        let (x0, y0) = (ox + col * (patch + gap), oy + row * (patch + gap));
        for y in y0..y0 + patch {
            for x in x0..x0 + patch {
                put(&mut buf, x, y, *c);
            }
        }
    }

    // Color wheel (hue = angle, saturation = radius)
    let (cx, cy, r) = (985.0f32, 255.0f32, 200.0f32);
    for y in (cy - r) as u32..(cy + r) as u32 {
        for x in (cx - r) as u32..(cx + r) as u32 {
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let d = (dx * dx + dy * dy).sqrt();
            if d < r {
                let h = dy.atan2(dx) / std::f32::consts::TAU;
                put(&mut buf, x, y, hsv(h, d / r, 0.95));
            }
        }
    }

    // Gray ramp and hue ramp
    for x in 0..W {
        let t = x as f32 / (W - 1) as f32;
        let g = (t * 255.0) as u8;
        for y in 530..590 {
            put(&mut buf, x, y, [g, g, g]);
        }
        for y in 610..680 {
            let v = 1.0 - (y - 610) as f32 / 70.0 * 0.6;
            put(&mut buf, x, y, hsv(t, 0.85, v));
        }
    }
    buf
}

fn draw_sun(buf: &mut [u8], t: f32) {
    let cx = W as f32 * (0.5 + 0.42 * (t * 0.35).sin());
    let cy = 470.0 + 25.0 * (t * 1.3).sin();
    let r = 55.0f32;
    let (x0, x1) = ((cx - r).max(0.0) as u32, ((cx + r) as u32).min(W - 1));
    let (y0, y1) = ((cy - r).max(0.0) as u32, ((cy + r) as u32).min(H - 1));
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let d = (dx * dx + dy * dy).sqrt() / r;
            if d < 1.0 {
                let a = (1.0 - d * d).powf(0.6);
                let i = ((y * W + x) * 4) as usize;
                let sun = [255.0, 236.0, 190.0];
                for c in 0..3 {
                    let v = buf[i + c] as f32;
                    buf[i + c] = (v + (sun[c] - v) * a) as u8;
                }
            }
        }
    }
}
