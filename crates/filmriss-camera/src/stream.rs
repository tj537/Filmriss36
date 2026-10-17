use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use crate::controls::{Command, HwControl, HwValue};
use crate::{CameraDescriptor, CameraId, Frame};

/// Called after every new frame (e.g. `egui::Context::request_repaint`).
pub type Notify = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Debug, PartialEq)]
pub enum StreamState {
    Starting,
    Running { width: u32, height: u32, format: String },
    Failed(String),
}

const POOL_SIZE: usize = 4;

pub(crate) struct Shared {
    latest: Mutex<Option<Frame>>,
    still: Mutex<Option<Result<Frame, String>>>,
    pool: Mutex<Vec<Vec<u8>>>,
    state: Mutex<StreamState>,
    controls: Mutex<Vec<HwControl>>,
    commands: Mutex<Vec<Command>>,
    stop: AtomicBool,
    seq: AtomicU64,
    notify: Notify,
}

impl Shared {
    pub(crate) fn should_stop(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    pub(crate) fn set_state(&self, state: StreamState) {
        if let StreamState::Failed(msg) = &state {
            log::error!("camera: {msg}");
        }
        *self.state.lock().unwrap() = state;
        (self.notify)();
    }

    pub(crate) fn take_commands(&self) -> Vec<Command> {
        std::mem::take(&mut *self.commands.lock().unwrap())
    }

    pub(crate) fn set_controls(&self, controls: Vec<HwControl>) {
        *self.controls.lock().unwrap() = controls;
        (self.notify)();
    }

    /// Gets a (recycled) buffer of exactly `len` bytes.
    pub(crate) fn buffer(&self, len: usize) -> Vec<u8> {
        let mut buf = self.pool.lock().unwrap().pop().unwrap_or_default();
        buf.resize(len, 0);
        buf
    }

    pub(crate) fn recycle(&self, buf: Vec<u8>) {
        let mut pool = self.pool.lock().unwrap();
        if pool.len() < POOL_SIZE {
            pool.push(buf);
        }
    }

    fn frame(&self, width: u32, height: u32, data: Vec<u8>) -> Frame {
        debug_assert_eq!(data.len(), (width * height * 4) as usize);
        Frame {
            width,
            height,
            data,
            seq: self.seq.fetch_add(1, Ordering::Relaxed) + 1,
            timestamp: Instant::now(),
        }
    }

    pub(crate) fn publish(&self, width: u32, height: u32, data: Vec<u8>) {
        let frame = self.frame(width, height, data);
        let dropped = self.latest.lock().unwrap().replace(frame);
        if let Some(old) = dropped {
            self.recycle(old.data);
        }
        (self.notify)();
    }

    /// Delivers the result of a [`Command::CaptureStill`].
    pub(crate) fn publish_still(&self, result: Result<(u32, u32, Vec<u8>), String>) {
        let still = result.map(|(w, h, data)| self.frame(w, h, data));
        *self.still.lock().unwrap() = Some(still);
        (self.notify)();
    }
}

/// A running camera stream. Dropping it stops the capture thread.
pub struct CameraStream {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    descriptor: CameraDescriptor,
}

impl CameraStream {
    /// Starts the stream in the background – does not block. Errors end up in [`Self::state`].
    pub fn open(descriptor: CameraDescriptor, notify: Notify) -> Self {
        let shared = Arc::new(Shared {
            latest: Mutex::new(None),
            still: Mutex::new(None),
            pool: Mutex::new(Vec::new()),
            state: Mutex::new(StreamState::Starting),
            controls: Mutex::new(Vec::new()),
            commands: Mutex::new(Vec::new()),
            stop: AtomicBool::new(false),
            seq: AtomicU64::new(0),
            notify,
        });
        let id = descriptor.id.clone();
        let thread_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("filmriss-camera".into())
            .spawn(move || match id {
                CameraId::TestPattern => crate::test_pattern::run(&thread_shared),
                #[cfg(feature = "native")]
                CameraId::Native(index) => crate::native::run(&index, &thread_shared),
                #[cfg(not(feature = "native"))]
                CameraId::Native(_) => {
                    thread_shared.set_state(StreamState::Failed("native cameras are disabled in this build".into()))
                }
            })
            .expect("failed to spawn camera thread");

        Self {
            shared,
            thread: Some(thread),
            descriptor,
        }
    }

    pub fn descriptor(&self) -> &CameraDescriptor {
        &self.descriptor
    }

    /// Takes the newest frame (if one arrived since the last call).
    pub fn take_frame(&self) -> Option<Frame> {
        self.shared.latest.lock().unwrap().take()
    }

    /// Returns a no-longer-needed frame buffer for reuse.
    pub fn recycle(&self, data: Vec<u8>) {
        self.shared.recycle(data);
    }

    pub fn state(&self) -> StreamState {
        self.shared.state.lock().unwrap().clone()
    }

    /// Asks the camera for one frame at its highest resolution. The preview pauses briefly
    /// while the camera switches formats. The result arrives via [`Self::take_still`].
    pub fn request_still(&self) {
        *self.shared.still.lock().unwrap() = None;
        self.shared.commands.lock().unwrap().push(Command::CaptureStill);
    }

    pub fn take_still(&self) -> Option<Result<Frame, String>> {
        self.shared.still.lock().unwrap().take()
    }

    /// Hardware controls the camera reports (may be empty).
    pub fn controls(&self) -> Vec<HwControl> {
        self.shared.controls.lock().unwrap().clone()
    }

    pub fn set_control(&self, id: usize, value: HwValue) {
        let mut commands = self.shared.commands.lock().unwrap();
        // Only the newest value per control matters (sliders send many updates).
        commands.retain(|c| !matches!(c, Command::SetControl(other, _) if *other == id));
        commands.push(Command::SetControl(id, value));
    }

    /// Back to camera automatic: every control to its default.
    pub fn reset_controls(&self) {
        self.shared.commands.lock().unwrap().push(Command::ResetControls);
    }
}

impl Drop for CameraStream {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
