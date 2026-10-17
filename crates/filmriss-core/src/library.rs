//! Library of all film simulations: built-in + custom recipes + `.cube` files.
//!
//! Folder layout (macOS: `~/Library/Application Support/de.Filmriss.Filmriss/`):
//! - `recipes/*.toml` – custom recipes (created in the app or written by hand)
//! - `luts/*.cube`    – third-party 3D LUTs; each one shows up as a simulation automatically
//! - `exports/*.cube` – LUTs exported from recipes for Lightroom, Resolve & co.
//!
//! Photos go to `~/Pictures/Filmriss/` (or the system's pictures folder).

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::lut::Lut3D;
use crate::process::LookParams;
use crate::recipe::FilmRecipe;
use crate::{DEFAULT_LUT_SIZE, presets};

#[derive(Clone, Debug)]
pub struct Paths {
    pub recipes: PathBuf,
    pub luts: PathBuf,
    pub exports: PathBuf,
    pub captures: PathBuf,
}

impl Paths {
    /// Platform-standard folders. `None` if the system provides none (e.g. Android –
    /// there the path will come from the app context, see [`Paths::from_root`]).
    pub fn discover() -> Option<Self> {
        let proj = directories::ProjectDirs::from("de", "Filmriss", "Filmriss")?;
        let data = proj.data_dir();
        let captures = directories::UserDirs::new()
            .and_then(|u| u.picture_dir().map(|p| p.join("Filmriss")))
            .unwrap_or_else(|| data.join("captures"));
        Some(Self {
            recipes: data.join("recipes"),
            luts: data.join("luts"),
            exports: data.join("exports"),
            captures,
        })
    }

    pub fn from_root(root: &Path) -> Self {
        Self {
            recipes: root.join("recipes"),
            luts: root.join("luts"),
            exports: root.join("exports"),
            captures: root.join("captures"),
        }
    }

    pub fn ensure_exist(&self) -> io::Result<()> {
        for dir in [&self.recipes, &self.luts, &self.exports, &self.captures] {
            fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Origin {
    BuiltIn,
    UserRecipe(PathBuf),
    CubeFile(PathBuf),
}

/// A selectable film simulation including its baked LUT.
#[derive(Clone, Debug)]
pub struct Simulation {
    pub recipe: FilmRecipe,
    pub origin: Origin,
    /// Optional base LUT (from a `.cube` file) the recipe builds on.
    pub base: Option<Arc<Lut3D>>,
    pub lut: Arc<Lut3D>,
}

impl Simulation {
    pub fn new(recipe: FilmRecipe, origin: Origin, base: Option<Arc<Lut3D>>) -> Self {
        let lut = Arc::new(recipe.bake(DEFAULT_LUT_SIZE, base.as_deref()));
        Self {
            recipe,
            origin,
            base,
            lut,
        }
    }

    pub fn name(&self) -> &str {
        &self.recipe.name
    }

    pub fn is_builtin(&self) -> bool {
        self.origin == Origin::BuiltIn
    }

    /// Re-bake after recipe changes (~1 ms at 33³).
    pub fn rebake(&mut self) {
        self.lut = Arc::new(self.recipe.bake(DEFAULT_LUT_SIZE, self.base.as_deref()));
    }

    pub fn look_params(&self, intensity: f32, seed: u32) -> LookParams {
        LookParams {
            intensity,
            grain: self.recipe.grain,
            grain_size: self.recipe.grain_size,
            grain_roughness: self.recipe.grain_roughness,
            // Colored grain makes no sense on black & white film.
            grain_chroma: if self.recipe.monochrome {
                0.0
            } else {
                self.recipe.grain_chroma
            },
            vignette: self.recipe.vignette,
            seed,
            pre_gain: [1.0; 3],
        }
    }
}

pub struct Library {
    pub simulations: Vec<Simulation>,
    /// Non-fatal problems while loading (broken file etc.) for the UI.
    pub warnings: Vec<String>,
}

impl Library {
    pub fn load(paths: Option<&Paths>) -> Self {
        let mut simulations: Vec<Simulation> = presets::builtin()
            .into_iter()
            .map(|(_, r)| Simulation::new(r, Origin::BuiltIn, None))
            .collect();
        let mut warnings = Vec::new();

        if let Some(paths) = paths {
            let mut cubes: HashMap<String, Arc<Lut3D>> = HashMap::new();

            for path in list_files(&paths.luts, "cube") {
                match load_cube(&path) {
                    Ok(lut) => {
                        let file_name = file_name(&path);
                        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                        let recipe = FilmRecipe {
                            name: stem.into_owned(),
                            description: format!("3D LUT from {file_name}"),
                            base_lut: Some(file_name.clone()),
                            ..FilmRecipe::default()
                        };
                        simulations.push(Simulation::new(recipe, Origin::CubeFile(path), Some(lut.clone())));
                        cubes.insert(file_name, lut);
                    }
                    Err(e) => warnings.push(format!("{}: {e}", path.display())),
                }
            }

            for path in list_files(&paths.recipes, "toml") {
                let recipe = match fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|t| FilmRecipe::from_toml(&t).map_err(|e| e.to_string()))
                {
                    Ok(r) => r,
                    Err(e) => {
                        warnings.push(format!("{}: {e}", path.display()));
                        continue;
                    }
                };
                let base = match &recipe.base_lut {
                    None => None,
                    Some(name) => match cubes.get(name) {
                        Some(lut) => Some(lut.clone()),
                        None => {
                            warnings.push(format!(
                                "{}: base LUT '{name}' not found in the LUT folder",
                                path.display()
                            ));
                            continue;
                        }
                    },
                };
                simulations.push(Simulation::new(recipe, Origin::UserRecipe(path), base));
            }
        }

        for w in &warnings {
            log::warn!("{w}");
        }
        Self { simulations, warnings }
    }

    /// Saves a recipe as TOML. If `existing` already belongs to a custom recipe,
    /// that file is overwritten, otherwise a new one is created.
    pub fn save_recipe(paths: &Paths, recipe: &FilmRecipe, existing: Option<&Path>) -> io::Result<PathBuf> {
        fs::create_dir_all(&paths.recipes)?;
        let path = match existing {
            Some(p) if p.starts_with(&paths.recipes) => p.to_path_buf(),
            _ => unique_path(&paths.recipes, &slug(&recipe.name), "toml"),
        };
        let header = "# Filmriss recipe – all fields optional, see presets/ in the repository\n";
        fs::write(&path, format!("{header}{}", recipe.to_toml()))?;
        Ok(path)
    }

    /// Exports a simulation's baked LUT as `.cube`.
    pub fn export_cube(paths: &Paths, sim: &Simulation) -> io::Result<PathBuf> {
        fs::create_dir_all(&paths.exports)?;
        let path = unique_path(&paths.exports, &slug(sim.name()), "cube");
        fs::write(&path, sim.lut.to_cube_string(sim.name()))?;
        Ok(path)
    }
}

pub fn load_cube(path: &Path) -> Result<Arc<Lut3D>, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    Lut3D::parse_cube(&text).map(Arc::new).map_err(|e| e.to_string())
}

fn list_files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)))
        .collect();
    files.sort();
    files
}

fn file_name(path: &Path) -> String {
    path.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut path = dir.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem}-{n}.{ext}"));
        n += 1;
    }
    path
}

/// File-system friendly name: "My Grün-Look!" → "my-gruen-look" (German umlauts are transliterated).
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        match c {
            'ä' => out.push_str("ae"),
            'ö' => out.push_str("oe"),
            'ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            c if c.is_ascii_alphanumeric() => out.push(c),
            _ if !out.ends_with('-') => out.push('-'),
            _ => {}
        }
    }
    let out = out.trim_matches('-').to_owned();
    if out.is_empty() { "recipe".into() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_handles_umlauts() {
        assert_eq!(slug("My Grün-Look!"), "my-gruen-look");
        assert_eq!(slug("???"), "recipe");
    }

    #[test]
    fn load_save_roundtrip() {
        let root = std::env::temp_dir().join(format!("filmriss-test-{}", std::process::id()));
        let paths = Paths::from_root(&root);
        paths.ensure_exist().unwrap();
        fs::write(
            paths.luts.join("warm.cube"),
            Lut3D::from_fn(5, |[r, g, b]| [r, g, b * 0.8]).to_cube_string("warm"),
        )
        .unwrap();
        let recipe = FilmRecipe {
            name: "My Look".into(),
            contrast: 0.2,
            base_lut: Some("warm.cube".into()),
            ..Default::default()
        };
        let saved = Library::save_recipe(&paths, &recipe, None).unwrap();
        assert!(saved.ends_with("my-look.toml"));

        let lib = Library::load(Some(&paths));
        assert!(lib.warnings.is_empty(), "{:?}", lib.warnings);
        let builtin = presets::builtin().len();
        assert_eq!(lib.simulations.len(), builtin + 2);
        let mine = lib.simulations.iter().find(|s| s.name() == "My Look").unwrap();
        assert!(mine.base.is_some());
        fs::remove_dir_all(&root).unwrap();
    }
}
