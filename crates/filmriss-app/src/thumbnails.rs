//! Mini previews for the film strip: the current camera frame with every
//! simulation. Runs on the CPU (tiny images, < 2 ms for all of them) and is only
//! refreshed about once per second.

use egui::{ColorImage, TextureHandle, TextureOptions};
use filmriss_core::{LookParams, Simulation, process};

use crate::preview::PreviewFrame;

const THUMB_W: u32 = 128;
const REFRESH_SECS: f64 = 1.0;

#[derive(Default)]
pub struct Thumbnails {
    textures: Vec<TextureHandle>,
    last_update: f64,
    dirty: bool,
}

impl Thumbnails {
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    pub fn get(&self, i: usize) -> Option<&TextureHandle> {
        self.textures.get(i)
    }

    pub fn update(&mut self, ctx: &egui::Context, now: f64, frame: Option<&PreviewFrame>, sims: &[Simulation]) {
        let Some(frame) = frame else { return };
        let stale = now - self.last_update > REFRESH_SECS || self.textures.len() != sims.len();
        if !(stale || self.dirty) {
            return;
        }
        self.last_update = now;
        self.dirty = false;

        let f = &frame.frame;
        let th = ((THUMB_W as f32 * f.height as f32 / f.width as f32).round() as u32).max(1);
        let small = process::downscale_rgba8(&f.data, f.width, f.height, THUMB_W, th);
        let size = [THUMB_W as usize, th as usize];

        self.textures.truncate(sims.len());
        for (i, sim) in sims.iter().enumerate() {
            let mut img = small.clone();
            // Grain would just be noise at this size – vignette and colors are enough.
            let params = LookParams {
                grain: 0.0,
                ..sim.look_params(1.0, 0)
            };
            process::render_rgba8(&mut img, THUMB_W, th, &sim.lut, &params);
            let image = ColorImage::from_rgba_unmultiplied(size, &img);
            match self.textures.get_mut(i) {
                Some(tex) => tex.set(image, TextureOptions::LINEAR),
                None => self
                    .textures
                    .push(ctx.load_texture(format!("thumb-{i}"), image, TextureOptions::LINEAR)),
            }
        }
    }
}
