//! Native Kameras über nokhwa (AVFoundation / V4L2 / Media Foundation).

use nokhwa::pixel_format::RgbAFormat;
use nokhwa::utils::{ApiBackend, CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType};
use nokhwa::{Camera, FormatDecoder as _};

use crate::stream::{Shared, StreamState};
use crate::{CameraDescriptor, CameraId, NativeIndex};

/// Nach so vielen Fehlern in Folge gilt der Stream als tot.
const MAX_CONSECUTIVE_ERRORS: u32 = 60;

pub(crate) fn list() -> Vec<CameraDescriptor> {
    match nokhwa::query(ApiBackend::Auto) {
        Ok(cams) => cams
            .into_iter()
            .map(|info| CameraDescriptor {
                id: CameraId::Native(match info.index() {
                    CameraIndex::Index(i) => NativeIndex::Index(*i),
                    CameraIndex::String(s) => NativeIndex::Name(s.clone()),
                }),
                name: info.human_name(),
            })
            .collect(),
        Err(e) => {
            log::warn!("Kameras konnten nicht abgefragt werden: {e}");
            Vec::new()
        }
    }
}

pub(crate) fn run(index: &NativeIndex, shared: &Shared) {
    let mut cam = match open(index) {
        Ok(cam) => cam,
        Err(e) => {
            shared.set_state(StreamState::Failed(format!("Kamera konnte nicht geöffnet werden: {e}")));
            return;
        }
    };
    let fmt = cam.camera_format();
    log::info!("Kamera läuft: {fmt}");
    shared.set_state(StreamState::Running {
        width: fmt.width(),
        height: fmt.height(),
        format: format!("{} @ {} fps", fmt.format(), fmt.frame_rate()),
    });

    let mut errors = 0u32;
    while !shared.should_stop() {
        let result = cam.frame().and_then(|buf| {
            let res = buf.resolution();
            let (w, h) = (res.width(), res.height());
            let mut data = shared.buffer((w * h * 4) as usize);
            match buf.decode_image_to_buffer::<RgbAFormat>(&mut data) {
                Ok(()) => Ok((w, h, data)),
                Err(e) => {
                    shared.recycle(data);
                    Err(e)
                }
            }
        });
        match result {
            Ok((w, h, data)) => {
                errors = 0;
                shared.publish(w, h, data);
            }
            Err(e) => {
                errors += 1;
                log::warn!("Kamerabild verworfen: {e}");
                if errors >= MAX_CONSECUTIVE_ERRORS {
                    shared.set_state(StreamState::Failed(format!("Kamera liefert keine Bilder: {e}")));
                    break;
                }
            }
        }
    }
    let _ = cam.stop_stream();
}

fn open(index: &NativeIndex) -> Result<Camera, nokhwa::NokhwaError> {
    let index = match index {
        NativeIndex::Index(i) => CameraIndex::Index(*i),
        NativeIndex::Name(s) => CameraIndex::String(s.clone()),
    };
    let mut cam = Camera::new(index, RequestedFormat::new::<RgbAFormat>(RequestedFormatType::None))?;

    // Eigene Formatwahl: flüssig (≥ 24 fps) schlägt Auflösung, Ziel ist ~1080p.
    match cam.compatible_camera_formats() {
        Ok(formats) => {
            if let Some(best) = pick_format(&formats) {
                let request = RequestedFormat::new::<RgbAFormat>(RequestedFormatType::Exact(best));
                if let Err(e) = cam.set_camera_requset(request) {
                    log::warn!("Format {best} nicht setzbar, nutze Standard: {e}");
                }
            }
        }
        Err(e) => log::warn!("Kameraformate nicht abfragbar: {e}"),
    }
    cam.open_stream()?;
    Ok(cam)
}

fn pick_format(formats: &[CameraFormat]) -> Option<CameraFormat> {
    const TARGET: i64 = 1920 * 1080;
    formats
        .iter()
        .filter(|f| RgbAFormat::FORMATS.contains(&f.format()))
        .max_by_key(|f| {
            let pixels = f.width() as i64 * f.height() as i64;
            let smooth = f.frame_rate() >= 24;
            // Unkomprimierte Formate sind billiger zu dekodieren als MJPEG.
            let cheap = matches!(f.format(), FrameFormat::NV12 | FrameFormat::YUYV);
            (smooth, -(pixels - TARGET).abs(), f.frame_rate(), cheap)
        })
        .copied()
}
