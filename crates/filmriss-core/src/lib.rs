//! Filmriss Core – platform-independent image logic.
//!
//! This crate knows neither camera nor GPU. It provides:
//! - [`lut::Lut3D`] – 3D LUTs (load/save in `.cube` format, trilinear sampling)
//! - [`recipe::FilmRecipe`] – parametric film simulations that are "baked" into a 3D LUT
//! - [`presets`] – the built-in simulations
//! - [`library`] – management of built-in + custom simulations on disk
//! - [`process`] – CPU reference pipeline (for photos and thumbnails), identical to the GPU shader

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

/// Default resolution for baked LUTs (33³ is the industry standard).
pub const DEFAULT_LUT_SIZE: usize = 33;
