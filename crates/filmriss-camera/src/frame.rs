use std::time::Instant;

/// A camera frame as RGBA8 (sRGB-encoded, no row padding).
#[derive(Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    /// Sequential frame number since the stream started.
    pub seq: u64,
    pub timestamp: Instant,
}
