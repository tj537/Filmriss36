//! Fotoaufnahme: volles Kamerabild durch die CPU-Pipeline (identisch zur GPU-Preview)
//! und als JPEG speichern. Läuft in einem eigenen Thread, die UI bleibt flüssig.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Sender;

use anyhow::Context as _;
use filmriss_camera::Frame;
use filmriss_core::{LookParams, Lut3D, library::slug, process};

pub struct CaptureJob {
    pub frame: Arc<Frame>,
    pub lut: Arc<Lut3D>,
    pub params: LookParams,
    pub simulation: String,
    pub dir: PathBuf,
    pub save_original: bool,
}

pub type CaptureResult = Result<PathBuf, String>;

pub fn spawn(job: CaptureJob, done: Sender<CaptureResult>, ctx: egui::Context) {
    std::thread::Builder::new()
        .name("filmriss-capture".into())
        .spawn(move || {
            let result = run(&job).map_err(|e| format!("{e:#}"));
            let _ = done.send(result);
            ctx.request_repaint();
        })
        .expect("Capture-Thread konnte nicht gestartet werden");
}

fn run(job: &CaptureJob) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(&job.dir).with_context(|| format!("Ordner {} anlegen", job.dir.display()))?;
    let stamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let base = format!("FR_{stamp}_{}", slug(&job.simulation));
    let f = &job.frame;

    let t = std::time::Instant::now();
    let mut pixels = f.data.clone();
    process::render_rgba8(&mut pixels, f.width, f.height, &job.lut, &job.params);
    log::info!("Look auf {}×{} angewendet in {:?}", f.width, f.height, t.elapsed());

    let path = unique(&job.dir, &base, "jpg");
    write_jpeg(&path, &pixels, f.width, f.height)?;
    if job.save_original {
        write_jpeg(
            &unique(&job.dir, &format!("{base}_original"), "jpg"),
            &f.data,
            f.width,
            f.height,
        )?;
    }
    Ok(path)
}

fn write_jpeg(path: &Path, rgba: &[u8], width: u32, height: u32) -> anyhow::Result<()> {
    let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let file = std::fs::File::create(path).with_context(|| format!("{} anlegen", path.display()))?;
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(file), 92);
    encoder
        .encode(&rgb, width, height, image::ExtendedColorType::Rgb8)
        .context("JPEG kodieren")?;
    Ok(())
}

fn unique(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut path = dir.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem}_{n}.{ext}"));
        n += 1;
    }
    path
}
