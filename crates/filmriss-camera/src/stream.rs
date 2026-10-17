use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

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
    pool: Mutex<Vec<Vec<u8>>>,
    state: Mutex<StreamState>,
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

    pub(crate) fn publish(&self, width: u32, height: u32, data: Vec<u8>) {
        debug_assert_eq!(data.len(), (width * height * 4) as usize);
        let frame = Frame {
            width,
            height,
            data,
            seq: self.seq.fetch_add(1, Ordering::Relaxed) + 1,
            timestamp: Instant::now(),
        };
        let dropped = self.latest.lock().unwrap().replace(frame);
        if let Some(old) = dropped {
            self.recycle(old.data);
        }
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
            pool: Mutex::new(Vec::new()),
            state: Mutex::new(StreamState::Starting),
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
}

impl Drop for CameraStream {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
