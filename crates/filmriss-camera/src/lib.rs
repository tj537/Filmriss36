//! Camera abstraction for Filmriss.
//!
//! Every source runs on its own capture thread and only ever puts the *newest*
//! frame into a slot ([`CameraStream::take_frame`]). If the UI can't keep up, old
//! frames are dropped instead of queued – keeping live-preview latency minimal.
//! Frame buffers are recycled (no 8 MB allocation per frame).
//!
//! Backends:
//! - [`CameraId::TestPattern`]: animated test pattern (available everywhere, ideal for developing looks)
//! - [`CameraId::Native`] (feature `native`): nokhwa → AVFoundation / V4L2 / MSMF
//! - planned: Android (Camera2 NDK), Raspberry Pi (libcamera) – see `docs/ARCHITECTURE.md`

mod controls;
mod frame;
#[cfg(feature = "native")]
mod native;
mod stream;
mod test_pattern;

pub use controls::{HwControl, HwControlKind, HwValue};
pub use frame::Frame;
pub use stream::{CameraStream, Notify, StreamState};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CameraId {
    TestPattern,
    Native(NativeIndex),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum NativeIndex {
    Index(u32),
    Name(String),
}

impl CameraId {
    /// Stable key used to remember the last used camera.
    pub fn key(&self) -> String {
        match self {
            CameraId::TestPattern => "test".into(),
            CameraId::Native(NativeIndex::Index(i)) => format!("idx:{i}"),
            CameraId::Native(NativeIndex::Name(n)) => format!("name:{n}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CameraDescriptor {
    pub id: CameraId,
    pub name: String,
}

impl CameraDescriptor {
    pub fn test_pattern() -> Self {
        Self {
            id: CameraId::TestPattern,
            name: "Test pattern".into(),
        }
    }
}

/// Requests camera permission (on macOS). The callback arrives asynchronously;
/// on other platforms it is called immediately with `true`.
pub fn request_permission(on_done: impl Fn(bool) + Send + Sync + 'static) {
    #[cfg(feature = "native")]
    nokhwa::nokhwa_initialize(on_done);
    #[cfg(not(feature = "native"))]
    on_done(true);
}

/// All available cameras; the test pattern is always included as the last entry.
pub fn list_cameras() -> Vec<CameraDescriptor> {
    #[allow(unused_mut)]
    let mut list = Vec::new();
    #[cfg(feature = "native")]
    list.extend(native::list());
    list.push(CameraDescriptor::test_pattern());
    list
}
