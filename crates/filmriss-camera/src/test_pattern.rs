//! Animated test pattern: ColorChecker patches, color wheel, gray and hue ramps and a
//! moving "sun". Lets you judge looks without a camera, and you immediately see
//! whether the preview stutters.
//!
//! It also behaves like a small fake camera so every feature can be tried without
//! hardware: the preview runs at 1280×720, full-resolution stills come out at
//! 2560×1440 ("sensor resolution"), and it offers simulated manual controls.

use std::time::{Duration, Instant};

use crate::controls::{Command, HwControl, HwControlKind, HwValue};
use crate::stream::{Shared, StreamState};

const W: u32 = 1280;
const H: u32 = 720;
/// Still captures are rendered at this multiple of the preview size.
const STILL_SCALE: u32 = 2;
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

/// Simulated manual controls (ids are indices into this list).
fn initial_controls() -> Vec<HwControl> {
    vec![
        HwControl {
            id: 0,
            name: "Auto exposure".into(),
            kind: HwControlKind::Bool {
                value: true,
                default: true,
            },
            read_only: false,
        },
        HwControl {
            id: 1,
            name: "Exposure time (ms)".into(),
            kind: HwControlKind::Int {
                min: 1,
                max: 100,
                step: 1,
                value: 33,
                default: 33,
            },
            read_only: false,
        },
        HwControl {
            id: 2,
            name: "Gain".into(),
            kind: HwControlKind::Int {
                min: 0,
                max: 100,
                step: 1,
                value: 0,
                default: 0,
            },
            read_only: false,
        },
    ]
}

/// Brightness lookup table emulating exposure time and gain.
fn exposure_table(controls: &[HwControl]) -> Option<[u8; 256]> {
    let value = |i: usize| match controls[i].kind.current_value() {
        HwValue::Int(v) | HwValue::Choice(v) => v as f32,
        HwValue::Float(v) => v as f32,
        HwValue::Bool(b) => b as u8 as f32,
    };
    if value(0) > 0.5 {
        return None;
    }
    let factor = value(1) / 33.0 * (1.0 + value(2) / 25.0);
    let mut table = [0u8; 256];
    for (i, t) in table.iter_mut().enumerate() {
        let lin = ((i as f32 / 255.0 + 0.055) / 1.055).powf(2.4) * factor;
        let srgb = 1.055 * lin.min(1.0).powf(1.0 / 2.4) - 0.055;
        *t = (srgb.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    }
    Some(table)
}

fn apply_table(buf: &mut [u8], table: &Option<[u8; 256]>) {
    if let Some(t) = table {
        for px in buf.chunks_exact_mut(4) {
            px[0] = t[px[0] as usize];
            px[1] = t[px[1] as usize];
            px[2] = t[px[2] as usize];
        }
    }
}

pub(crate) fn run(shared: &Shared) {
    let background = render_background(1);
    let mut controls = initial_controls();
    let mut table = None;
    shared.set_controls(controls.clone());
    shared.set_state(StreamState::Running {
        width: W,
        height: H,
        format: "test pattern".into(),
    });

    let start = Instant::now();
    let frame_time = Duration::from_secs_f64(1.0 / FPS as f64);
    let mut next = Instant::now();

    while !shared.should_stop() {
        let t = start.elapsed().as_secs_f32();
        for cmd in shared.take_commands() {
            match cmd {
                Command::CaptureStill => {
                    // A real camera needs a moment to switch formats – simulate that.
                    std::thread::sleep(Duration::from_millis(250));
                    let mut still = render_background(STILL_SCALE);
                    draw_sun(&mut still, STILL_SCALE, t);
                    apply_table(&mut still, &table);
                    shared.publish_still(Ok((W * STILL_SCALE, H * STILL_SCALE, still)));
                    next = Instant::now();
                }
                Command::SetControl(id, value) => {
                    if let Some(c) = controls.get_mut(id) {
                        set_value(&mut c.kind, value);
                    }
                    table = exposure_table(&controls);
                    shared.set_controls(controls.clone());
                }
                Command::ResetControls => {
                    controls = initial_controls();
                    table = None;
                    shared.set_controls(controls.clone());
                }
            }
        }

        let mut buf = shared.buffer(background.len());
        buf.copy_from_slice(&background);
        draw_sun(&mut buf, 1, t);
        apply_table(&mut buf, &table);
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

fn set_value(kind: &mut HwControlKind, value: HwValue) {
    match (kind, value) {
        (HwControlKind::Int { value: v, min, max, .. }, HwValue::Int(n)) => *v = n.clamp(*min, *max),
        (HwControlKind::Float { value: v, min, max, .. }, HwValue::Float(n)) => *v = n.clamp(*min, *max),
        (HwControlKind::Bool { value: v, .. }, HwValue::Bool(b)) => *v = b,
        (HwControlKind::Choice { value: v, .. }, HwValue::Choice(n)) => *v = n,
        _ => {}
    }
}

struct Canvas {
    buf: Vec<u8>,
    w: u32,
}

impl Canvas {
    fn put(&mut self, x: u32, y: u32, c: [u8; 3]) {
        let i = ((y * self.w + x) * 4) as usize;
        self.buf[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
    }
}

fn hsv(h: f32, s: f32, v: f32) -> [u8; 3] {
    let h6 = (h.rem_euclid(1.0)) * 6.0;
    let f = |n: f32| {
        let k = (n + h6) % 6.0;
        v - v * s * (k.min(4.0 - k).clamp(0.0, 1.0))
    };
    [(f(5.0) * 255.0) as u8, (f(3.0) * 255.0) as u8, (f(1.0) * 255.0) as u8]
}

/// Renders the static part at `scale` × 1280×720 (all coordinates are in 720p units).
fn render_background(scale: u32) -> Vec<u8> {
    let (w, h) = (W * scale, H * scale);
    let s = scale as f32;
    let mut cv = Canvas {
        buf: vec![0u8; (w * h * 4) as usize],
        w,
    };

    // Background: "sky" at the top → warm "ground" at the bottom
    for y in 0..h {
        let t = y as f32 / h as f32;
        let top = [70.0, 110.0, 160.0];
        let bottom = [150.0, 120.0, 90.0];
        let c = [0, 1, 2].map(|i| (top[i] + (bottom[i] - top[i]) * t) as u8);
        for x in 0..w {
            cv.put(x, y, c);
        }
    }

    // ColorChecker 6×4
    let (patch, gap, ox, oy) = (96 * scale, 12 * scale, 40 * scale, 40 * scale);
    for (i, c) in CHECKER.iter().enumerate() {
        let (col, row) = (i as u32 % 6, i as u32 / 6);
        let (x0, y0) = (ox + col * (patch + gap), oy + row * (patch + gap));
        for y in y0..y0 + patch {
            for x in x0..x0 + patch {
                cv.put(x, y, *c);
            }
        }
    }

    // Color wheel (hue = angle, saturation = radius)
    let (cx, cy, r) = (985.0 * s, 255.0 * s, 200.0 * s);
    for y in (cy - r) as u32..(cy + r) as u32 {
        for x in (cx - r) as u32..(cx + r) as u32 {
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let d = (dx * dx + dy * dy).sqrt();
            if d < r {
                let hue = dy.atan2(dx) / std::f32::consts::TAU;
                cv.put(x, y, hsv(hue, d / r, 0.95));
            }
        }
    }

    // Gray ramp and hue ramp
    for x in 0..w {
        let t = x as f32 / (w - 1) as f32;
        let g = (t * 255.0) as u8;
        for y in 530 * scale..590 * scale {
            cv.put(x, y, [g, g, g]);
        }
        for y in 610 * scale..680 * scale {
            let v = 1.0 - (y - 610 * scale) as f32 / (70.0 * s) * 0.6;
            cv.put(x, y, hsv(t, 0.85, v));
        }
    }
    cv.buf
}

fn draw_sun(buf: &mut [u8], scale: u32, t: f32) {
    let (w, h) = (W * scale, H * scale);
    let s = scale as f32;
    let cx = w as f32 * (0.5 + 0.42 * (t * 0.35).sin());
    let cy = (470.0 + 25.0 * (t * 1.3).sin()) * s;
    let r = 55.0 * s;
    let (x0, x1) = ((cx - r).max(0.0) as u32, ((cx + r) as u32).min(w - 1));
    let (y0, y1) = ((cy - r).max(0.0) as u32, ((cy + r) as u32).min(h - 1));
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let d = (dx * dx + dy * dy).sqrt() / r;
            if d < 1.0 {
                let a = (1.0 - d * d).powf(0.6);
                let i = ((y * w + x) * 4) as usize;
                let sun = [255.0, 236.0, 190.0];
                for c in 0..3 {
                    let v = buf[i + c] as f32;
                    buf[i + c] = (v + (sun[c] - v) * a) as u8;
                }
            }
        }
    }
}
