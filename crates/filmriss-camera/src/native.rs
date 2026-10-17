//! Native cameras via nokhwa (AVFoundation / V4L2 / Media Foundation).

use std::panic::{AssertUnwindSafe, catch_unwind};

use nokhwa::pixel_format::RgbAFormat;
use nokhwa::utils::{
    ApiBackend, CameraFormat, CameraIndex, ControlValueDescription, ControlValueSetter, FrameFormat,
    KnownCameraControl, KnownCameraControlFlag, RequestedFormat, RequestedFormatType,
};
use nokhwa::{Camera, FormatDecoder as _, NokhwaError};

use crate::controls::{Command, HwControl, HwControlKind, HwValue};
use crate::stream::{Shared, StreamState};
use crate::{CameraDescriptor, CameraId, NativeIndex};

/// After this many consecutive errors the stream is considered dead.
const MAX_CONSECUTIVE_ERRORS: u32 = 60;
/// Frames discarded after switching to the still format, so exposure can settle.
const STILL_WARMUP_FRAMES: usize = 5;

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
            log::warn!("could not query cameras: {e}");
            Vec::new()
        }
    }
}

pub(crate) fn run(index: &NativeIndex, shared: &Shared) {
    let mut cam = match open(index) {
        Ok(cam) => cam,
        Err(e) => {
            shared.set_state(StreamState::Failed(format!("could not open camera: {e}")));
            return;
        }
    };
    report_running(&cam, shared);
    let (mut known, mut controls) = read_controls(&cam);
    shared.set_controls(controls.clone());

    let mut errors = 0u32;
    while !shared.should_stop() {
        for cmd in shared.take_commands() {
            match cmd {
                Command::CaptureStill => {
                    shared.publish_still(capture_still(&mut cam));
                    if !cam.is_stream_open() {
                        shared.set_state(StreamState::Failed("camera did not resume after the photo".into()));
                        return;
                    }
                    report_running(&cam, shared);
                }
                Command::SetControl(id, value) => {
                    if let Some(&control) = known.get(id) {
                        set_control(&mut cam, control, value);
                    }
                    (known, controls) = read_controls(&cam);
                    shared.set_controls(controls.clone());
                }
                Command::ResetControls => {
                    for c in controls.iter().filter(|c| !c.read_only) {
                        if let Some(&control) = known.get(c.id) {
                            set_control(&mut cam, control, c.kind.default_value());
                        }
                    }
                    (known, controls) = read_controls(&cam);
                    shared.set_controls(controls.clone());
                }
            }
        }

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
                log::warn!("dropped camera frame: {e}");
                if errors >= MAX_CONSECUTIVE_ERRORS {
                    shared.set_state(StreamState::Failed(format!("camera delivers no frames: {e}")));
                    break;
                }
            }
        }
    }
    let _ = cam.stop_stream();
}

fn report_running(cam: &Camera, shared: &Shared) {
    let fmt = cam.camera_format();
    log::info!("camera running: {fmt}");
    shared.set_state(StreamState::Running {
        width: fmt.width(),
        height: fmt.height(),
        format: format!("{} @ {} fps", fmt.format(), fmt.frame_rate()),
    });
}

// ── Full-resolution stills ──────────────────────────────────────────────────

/// Switches to the largest format, grabs one frame and switches back to the preview format.
fn capture_still(cam: &mut Camera) -> Result<(u32, u32, Vec<u8>), String> {
    let preview = cam.camera_format();
    let best = cam.compatible_camera_formats().ok().and_then(|f| pick_still_format(&f));
    let Some(still_fmt) = best.filter(|b| b.resolution() != preview.resolution()) else {
        // The preview already runs at the highest resolution.
        return grab(cam).map_err(|e| e.to_string());
    };

    log::info!("switching to {still_fmt} for a full-resolution photo");
    let result = grab_with_format(cam, still_fmt);
    if let Err(e) = switch_format(cam, preview) {
        log::error!("could not return to the preview format: {e}");
    }
    result.map_err(|e| e.to_string())
}

fn grab_with_format(cam: &mut Camera, fmt: CameraFormat) -> Result<(u32, u32, Vec<u8>), NokhwaError> {
    switch_format(cam, fmt)?;
    for _ in 0..STILL_WARMUP_FRAMES {
        cam.frame()?;
    }
    grab(cam)
}

fn switch_format(cam: &mut Camera, fmt: CameraFormat) -> Result<(), NokhwaError> {
    cam.stop_stream()?;
    cam.set_camera_requset(RequestedFormat::new::<RgbAFormat>(RequestedFormatType::Exact(fmt)))?;
    cam.open_stream()
}

fn grab(cam: &mut Camera) -> Result<(u32, u32, Vec<u8>), NokhwaError> {
    let buf = cam.frame()?;
    let res = buf.resolution();
    let (w, h) = (res.width(), res.height());
    let mut data = vec![0u8; (w * h * 4) as usize];
    buf.decode_image_to_buffer::<RgbAFormat>(&mut data)?;
    Ok((w, h, data))
}

fn pick_still_format(formats: &[CameraFormat]) -> Option<CameraFormat> {
    formats
        .iter()
        .filter(|f| RgbAFormat::FORMATS.contains(&f.format()))
        .max_by_key(|f| (f.width() as u64 * f.height() as u64, f.frame_rate()))
        .copied()
}

// ── Hardware controls ───────────────────────────────────────────────────────

/// Reads the controls the device reports. Some nokhwa backends panic (`todo!()`) instead
/// of returning an error, hence `catch_unwind`.
fn read_controls(cam: &Camera) -> (Vec<KnownCameraControl>, Vec<HwControl>) {
    let list = match catch_unwind(AssertUnwindSafe(|| cam.camera_controls())) {
        Ok(Ok(list)) => list,
        Ok(Err(e)) => {
            log::info!("camera reports no controls: {e}");
            return (Vec::new(), Vec::new());
        }
        Err(_) => {
            log::warn!("camera backend does not support controls");
            return (Vec::new(), Vec::new());
        }
    };

    let mut known = Vec::new();
    let mut controls = Vec::new();
    for c in list {
        let kind = match c.description().clone() {
            ControlValueDescription::IntegerRange {
                min,
                max,
                value,
                step,
                default,
            } if min < max => HwControlKind::Int {
                min,
                max,
                step: step.max(1),
                value,
                default,
            },
            ControlValueDescription::FloatRange {
                min,
                max,
                value,
                step,
                default,
            } if min < max => HwControlKind::Float {
                min,
                max,
                step,
                value,
                default,
            },
            ControlValueDescription::Boolean { value, default } => HwControlKind::Bool { value, default },
            ControlValueDescription::Enum {
                value,
                possible,
                default,
            } if possible.len() > 1 => HwControlKind::Choice {
                value,
                options: possible,
                default,
            },
            _ => continue,
        };
        let read_only = c
            .flag()
            .iter()
            .any(|f| matches!(f, KnownCameraControlFlag::ReadOnly | KnownCameraControlFlag::Disabled));
        controls.push(HwControl {
            id: known.len(),
            name: c.name().to_owned(),
            kind,
            read_only,
        });
        known.push(c.control());
    }
    (known, controls)
}

fn set_control(cam: &mut Camera, control: KnownCameraControl, value: HwValue) {
    let setter = match value {
        HwValue::Int(v) => ControlValueSetter::Integer(v),
        HwValue::Float(v) => ControlValueSetter::Float(v),
        HwValue::Bool(v) => ControlValueSetter::Boolean(v),
        HwValue::Choice(v) => ControlValueSetter::EnumValue(v),
    };
    match catch_unwind(AssertUnwindSafe(|| cam.set_camera_control(control, setter))) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => log::warn!("could not set {control}: {e}"),
        Err(_) => log::warn!("camera backend does not support setting {control}"),
    }
}

// ── Preview format ──────────────────────────────────────────────────────────

fn open(index: &NativeIndex) -> Result<Camera, nokhwa::NokhwaError> {
    let index = match index {
        NativeIndex::Index(i) => CameraIndex::Index(*i),
        NativeIndex::Name(s) => CameraIndex::String(s.clone()),
    };
    let mut cam = Camera::new(index, RequestedFormat::new::<RgbAFormat>(RequestedFormatType::None))?;

    // Own format choice: smooth (≥ 24 fps) beats resolution, target is ~1080p.
    match cam.compatible_camera_formats() {
        Ok(formats) => {
            if let Some(best) = pick_format(&formats) {
                let request = RequestedFormat::new::<RgbAFormat>(RequestedFormatType::Exact(best));
                if let Err(e) = cam.set_camera_requset(request) {
                    log::warn!("cannot set format {best}, using default: {e}");
                }
            }
        }
        Err(e) => log::warn!("cannot query camera formats: {e}"),
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
            // Uncompressed formats are cheaper to decode than MJPEG.
            let cheap = matches!(f.format(), FrameFormat::NV12 | FrameFormat::YUYV);
            (smooth, -(pixels - TARGET).abs(), f.frame_rate(), cheap)
        })
        .copied()
}
