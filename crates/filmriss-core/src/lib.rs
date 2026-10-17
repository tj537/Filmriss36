//! Filmriss Core – plattformunabhängige Bildlogik.
//!
//! Dieses Crate kennt weder Kamera noch GPU. Es liefert:
//! - [`lut::Lut3D`] – 3D-LUTs (Laden/Speichern im `.cube`-Format, trilineares Sampling)
//! - [`recipe::FilmRecipe`] – parametrische Filmsimulationen, die zu einer 3D-LUT "gebacken" werden
//! - [`presets`] – die eingebauten Simulationen
//! - [`library`] – Verwaltung eingebauter + eigener Simulationen auf der Platte
//! - [`process`] – CPU-Referenzpipeline (für Fotoaufnahmen und Thumbnails), identisch zum GPU-Shader

pub mod color;
pub mod library;
pub mod lut;
pub mod presets;
pub mod process;
pub mod recipe;

pub use library::{Library, Origin, Paths, Simulation};
pub use lut::Lut3D;
pub use process::LookParams;
pub use recipe::FilmRecipe;

/// Standard-Auflösung für gebackene LUTs (33³ ist der Industriestandard).
pub const DEFAULT_LUT_SIZE: usize = 33;
