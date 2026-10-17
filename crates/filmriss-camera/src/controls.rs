//! Hardware camera controls in a backend-independent form.
//!
//! Backends report whatever the device exposes (exposure time, gain/ISO, white balance,
//! focus, …) – the UI renders them generically. What is available differs a lot:
//! V4L2 (Linux/Raspberry Pi) usually exposes everything, Mac cameras only very little
//! (Apple does not allow manual shutter/ISO there), Android Camera2 will expose full manual control.

#[derive(Clone, Debug, PartialEq)]
pub struct HwControl {
    /// Backend-specific index, pass it back to [`crate::CameraStream::set_control`].
    pub id: usize,
    pub name: String,
    pub kind: HwControlKind,
    pub read_only: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HwControlKind {
    Int {
        min: i64,
        max: i64,
        step: i64,
        value: i64,
        default: i64,
    },
    Float {
        min: f64,
        max: f64,
        step: f64,
        value: f64,
        default: f64,
    },
    Bool {
        value: bool,
        default: bool,
    },
    Choice {
        value: i64,
        options: Vec<i64>,
        default: i64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HwValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Choice(i64),
}

impl HwControlKind {
    pub fn default_value(&self) -> HwValue {
        match self {
            HwControlKind::Int { default, .. } => HwValue::Int(*default),
            HwControlKind::Float { default, .. } => HwValue::Float(*default),
            HwControlKind::Bool { default, .. } => HwValue::Bool(*default),
            HwControlKind::Choice { default, .. } => HwValue::Choice(*default),
        }
    }

    pub fn current_value(&self) -> HwValue {
        match self {
            HwControlKind::Int { value, .. } => HwValue::Int(*value),
            HwControlKind::Float { value, .. } => HwValue::Float(*value),
            HwControlKind::Bool { value, .. } => HwValue::Bool(*value),
            HwControlKind::Choice { value, .. } => HwValue::Choice(*value),
        }
    }
}

/// Commands from the UI to the capture thread.
#[derive(Clone, Debug)]
pub(crate) enum Command {
    /// Capture one frame at the highest resolution the camera offers.
    CaptureStill,
    SetControl(usize, HwValue),
    /// Set every writable control back to its default (= camera automatic).
    ResetControls,
}
