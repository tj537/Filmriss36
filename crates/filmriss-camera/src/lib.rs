//! Kamera-Abstraktion für Filmriss.
//!
//! Jede Quelle läuft in einem eigenen Capture-Thread und legt immer nur das
//! *neueste* Bild in einen Slot ([`CameraStream::take_frame`]). Kommt die UI nicht
//! hinterher, werden alte Bilder verworfen statt aufgestaut – so bleibt die Latenz
//! der Live-Preview minimal. Bildpuffer werden recycelt (keine 8-MB-Allokation pro Frame).
//!
//! Backends:
//! - [`CameraId::TestPattern`]: animiertes Testbild (überall verfügbar, ideal zum Entwickeln von Looks)
//! - [`CameraId::Native`] (Feature `native`): nokhwa → AVFoundation / V4L2 / MSMF
//! - geplant: Android (Camera2 NDK), Raspberry Pi (libcamera) – siehe `docs/ARCHITECTURE.md`

mod frame;
#[cfg(feature = "native")]
mod native;
mod stream;
mod test_pattern;

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
    /// Stabiler Schlüssel, um die zuletzt genutzte Kamera zu speichern.
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
            name: "Testbild".into(),
        }
    }
}

/// Fragt (auf macOS) die Kamera-Berechtigung an. Der Callback kommt asynchron,
/// auf anderen Plattformen sofort mit `true`.
pub fn request_permission(on_done: impl Fn(bool) + Send + Sync + 'static) {
    #[cfg(feature = "native")]
    nokhwa::nokhwa_initialize(on_done);
    #[cfg(not(feature = "native"))]
    on_done(true);
}

/// Alle verfügbaren Kameras; das Testbild ist immer als letzter Eintrag dabei.
pub fn list_cameras() -> Vec<CameraDescriptor> {
    #[allow(unused_mut)]
    let mut list = Vec::new();
    #[cfg(feature = "native")]
    list.extend(native::list());
    list.push(CameraDescriptor::test_pattern());
    list
}
