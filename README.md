# Filmriss 🎞️

A fast, open camera app with **live film simulations** – written in Rust.
Runs on macOS, Linux (including Raspberry Pi) and Windows; Android is prepared.

- **GPU live preview** (wgpu: Metal/Vulkan/DX12/GL) – film looks as 3D LUTs, one texture fetch per pixel
- **15 built-in simulations** inspired by classic film stocks (slide, negative, cinema, black & white …)
- **Custom simulations**: recipe editor with live preview, saved as readable TOML
- **Import `.cube` LUTs** (just drop them into the LUT folder) and **export** them (for Lightroom, Resolve & co.)
- **Film grain per simulation**: amount, size, roughness and color grain – resolution-independent, identical in preview and photo, with a quick grain slider next to the shutter
- **Full sensor resolution photos**: the preview runs smoothly at ~1080p, the shutter grabs the camera's largest format
- **Exposure modes AUTO / A / M**: exposure compensation and white balance everywhere, plus the camera's hardware controls in M (where the camera exposes them)
- **Touch and mouse**: swipe to switch, hold for before/after, large controls

> Status: early development version. Primary test target is macOS.

## Quick start (macOS)

```bash
# 1. Install Rust (if you haven't already)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 2. Run
git clone https://github.com/tj537/filmriss36.git
cd filmriss36
cargo run            # debug build (dependencies are still optimized → smooth)
cargo run --release  # maximum performance
```

On first launch macOS asks for **camera access** – the permission applies to the app you run
`cargo run` from (Terminal, iTerm, VS Code …). If you denied it by accident:
*System Settings › Privacy & Security › Camera*. Without a camera an animated test pattern is shown.

## Controls

| Action | Mouse / trackpad | Touch | Keyboard |
|---|---|---|---|
| Switch simulation | click/drag the film strip, two-finger swipe | swipe | ← / → |
| Take photo | shutter button | shutter button | Space |
| Show original | hold "Before" / hold on the preview | press and hold | hold C |
| Split view | "◑ Compare" | "◑ Compare" | S |
| Hide UI | double-click | double-tap | H |
| Technical info (FPS, format, GPU) | "ℹ Info" | | I |
| Exposure mode AUTO / A / M | mode buttons next to the shutter | same | E |

Photos are saved as JPEG to `~/Pictures/Filmriss/` at full sensor resolution (optionally together with the
original). Switch to *preview resolution* in the Camera tab if you prefer instant captures.

## Exposure modes

Laptop and phone cameras have a fixed aperture, so the modes work like this:

| Mode | What you control |
|---|---|
| **AUTO** | nothing – the camera handles exposure and white balance |
| **A** | the camera keeps auto-exposing; you set exposure compensation (EV) and white balance |
| **M** | like A, plus every hardware control the camera reports (exposure time, gain/ISO, focus, white balance …) |

EV and white balance are applied digitally in linear light before the film look, so they work on every
camera. Hardware controls depend on the platform: Linux/Raspberry Pi (V4L2) usually exposes a lot, **Mac
cameras expose almost nothing** (Apple doesn't allow manual shutter/ISO there), Android will get full
manual control via Camera2. The built-in test pattern simulates exposure time and gain so M mode can be
tried without hardware.

## Film grain

Every simulation has its own grain settings (Recipe tab → *Grain*):

| Parameter | Meaning |
|---|---|
| `grain` | amount, 0 … 1 (quick presets: Off / Weak / Strong) |
| `grain_size` | size in 1080p pixels, scales with the image so preview and 4K photo match (Small / Large) |
| `grain_roughness` | 0 = soft, round grain · 1 = crisp, crunchy grain |
| `grain_chroma` | 0 = monochrome grain (B&W film) · 1 = colored grain (color negative) |

The *Grain* slider next to the shutter changes the amount of the current simulation directly.

## Custom film simulations

**In the editor:** open the *Recipe* tab, move the sliders (applied live), enter a name and
click *"Save as new simulation"*. Double-click a slider to reset it.

**As a file:** recipes are TOML files in the recipe folder
(`~/Library/Application Support/de.Filmriss.Filmriss/recipes/`). All fields are optional:

```toml
name = "My Kodak Evening"
description = "Warm, soft, slightly faded"
temperature = 0.12
contrast = 0.1
fade = 0.1
saturation = 0.1
grain = 0.25
grain_roughness = 0.4
grain_chroma = 0.3

[hues.orange]          # skin tones
lightness = 0.1

[highlight_tone]       # split toning of the highlights
hue = 70.0             # Oklch hue in degrees (≈ 70 = amber, 200 = cyan)
strength = 0.3
```

The built-in recipes in [`crates/filmriss-core/presets/`](crates/filmriss-core/presets) make good templates.

**`.cube` LUTs:** copy them into the LUT folder (`…/de.Filmriss.Filmriss/luts/`) and click
*"Reload library"*. Each LUT appears as its own simulation and can be fine-tuned in the
editor – the result is saved as a recipe that references the LUT.

## Technology

Rust · egui/eframe · wgpu · nokhwa · Oklab color space · 3D LUTs (33³, RGBA16F)

Details on the pipeline, the color model and the road to Android & Raspberry Pi: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)

```
crates/filmriss-core     color math, LUTs, recipes, presets (no GPU/camera, tested)
crates/filmriss-camera   camera threads, test pattern, native backends
crates/filmriss-app      egui app, wgpu shader, editor, capture
```

```bash
cargo test -p filmriss-core   # color pipeline tests
cargo clippy --workspace
```

## License

MIT or Apache-2.0, at your option.
