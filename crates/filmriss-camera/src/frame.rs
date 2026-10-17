use std::time::Instant;

/// Ein Kamerabild als RGBA8 (sRGB-kodiert, ohne Zeilen-Padding).
#[derive(Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    /// Fortlaufende Bildnummer seit Start des Streams.
    pub seq: u64,
    pub timestamp: Instant,
}
