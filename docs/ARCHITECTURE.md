# Filmriss – Architecture

## Why Rust + wgpu + 3D LUTs?

| Requirement | Solution |
|---|---|
| Runs on macOS, Linux/Raspberry Pi, Windows, later Android | **Rust** compiles natively for all targets, no runtime/GC |
| Smooth live preview, even on weak hardware | **wgpu** (Metal / Vulkan / DX12 / OpenGL ES): all image processing happens in the fragment shader |
| Arbitrarily complex film looks without a performance hit | **3D LUT as a 3D texture**: the texture hardware interpolates trilinearly; a look costs exactly *one* texture fetch per pixel, no matter how elaborate its computation was |
| Create your own simulations | **Recipes** (TOML) with understandable sliders that get "baked" into a LUT |
| Use third-party looks | **`.cube` import** (Adobe/Resolve standard); recipes can build on top of a `.cube` |
| One UI for mouse and touch | **egui**: immediate-mode UI, touch events are handled like pointer events; runs on desktop and Android |

The "recipe → bake LUT → GPU" combination is the same idea cameras use internally: the
expensive color math happens **once** (≈1 ms for 33³ grid points), not 30 times per second
for each of 2 million pixels.

## Data flow

```
 Camera thread                         UI thread (egui)                    GPU
┌──────────────┐  newest frame    ┌───────────────────────┐  write_texture ┌──────────────────────┐
│ nokhwa /     │ ───────────────▶ │ take_frame()          │ ─────────────▶ │ camera texture RGBA8 │
│ test pattern │   (slot, old     │ Arc<Frame> + ID       │                │                      │
│ YUV→RGBA     │    frames are    │                       │  on change     │ LUT texture 33³ RGBA16F│
└──────────────┘    dropped)      │ select simulation ────┼──────────────▶ │                      │
        ▲                         │                       │                │ fragment shader:     │
        └── buffer recycling ◀────│ old frame unused?     │                │  LUT → intensity →   │
                                  └───────────────────────┘                │  vignette → grain    │
                                             │ shutter                     └──────────────────────┘
                                             ▼
                                  capture thread: same pipeline on the CPU (rayon) → JPEG
```

- **Latency**: the camera thread overwrites a single-frame slot. If the UI can't keep up,
  no backlog ever builds up.
- **Memory**: frame buffers are recycled (pool), no 8 MB allocation per frame.
- **Repaint only when needed**: the camera thread wakes egui via `request_repaint()`; without
  new frames the app sleeps.
- **Full-resolution stills**: the UI sends a `CaptureStill` command to the capture thread. The
  nokhwa backend stops the stream, switches to the largest decodable format, discards a few frames
  so exposure settles, grabs one frame and switches back to the preview format. If the preview already
  runs at the maximum resolution, the next frame is used directly. On error or timeout the app falls
  back to the preview frame. (On Android this becomes a separate still `ImageReader` – no switching.)
- **Camera controls**: backends report their hardware controls in a generic form
  (`HwControl`: int/float range, bool, choice) and the UI renders them in M mode. Setting a value is a
  command to the capture thread. Some nokhwa backends `todo!()` instead of erroring, so control calls are
  wrapped in `catch_unwind`.
- **Preview = photo**: `filmriss_core::process::shade` and `preview.wgsl` implement the same
  formulas (including an identical integer hash for the grain). The saved photo looks like the preview.

## Crates

```
crates/
├── filmriss-core     platform-independent, no GPU/camera – fully testable
│   ├── color.rs      sRGB transfer curves, Oklab
│   ├── lut.rs        Lut3D: read/write .cube, trilinear sampling
│   ├── recipe.rs     FilmRecipe: parameters → bake() → Lut3D
│   ├── presets.rs    built-in recipes (presets/*.toml, embedded at compile time)
│   ├── library.rs    built-in + custom recipes + .cube files, folder management
│   └── process.rs    CPU reference pipeline (photos, thumbnails)
├── filmriss-camera   capture threads, frame slot, buffer pool
│   ├── test_pattern  animated test pattern (ColorChecker, color wheel, ramps)
│   └── native        nokhwa: AVFoundation (macOS), V4L2 (Linux/Pi), MSMF (Windows)
└── filmriss-app      egui/eframe app
    ├── preview.rs    wgpu pipeline as an egui paint callback
    ├── preview.wgsl  the shader
    ├── app.rs        state, gestures, layout
    ├── editor.rs     recipe editor
    ├── thumbnails.rs film strip previews
    └── capture.rs    photo capture
```

## The recipe color model

Order in `FilmRecipe::apply` (input/output: sRGB-encoded, like camera frames):

1. sRGB → linear; **exposure** and **white balance** (channel gains, luminance-neutral)
2. **Black & white** via a channel mixer (simulates red, yellow, green … filters)
3. linear → **Oklab/Oklch** (perceptually uniform)
4. **Color ranges**: 7 bands (red, orange, yellow, green, cyan, blue, magenta) with smooth
   transitions – hue, saturation, lightness; near-neutral colors stay untouched
5. global **saturation**
6. **Tone curve** on L: contrast (power S-curve around mid gray), highlights, shadows, fade;
   chroma partly follows the lightness change (as with real film)
7. **Split toning** for shadows/highlights
8. back to sRGB with **gamut mapping** (reduce chroma instead of hard clipping)

Grain and vignette are part of the recipe but are *not* baked into the LUT (they depend on the
pixel position); they are computed in the shader.

Before the LUT, the shader applies the **camera gains** of modes A/M (exposure compensation and white
balance, in linear light). They belong to the capture, not to the look – like on a real camera.

### Grain

Grain is two octaves of value noise driven by an integer hash, identical on GPU and CPU:
- `grain_roughness` blends in the finer octave (soft, round → crisp, crunchy),
- `grain_chroma` mixes three independent per-channel fields into the monochrome one,
- the amplitude peaks in the midtones and falls off towards shadows/highlights,
- the size is given in 1080p pixels and scaled by `height / 1080`, so a 1080p preview and a 4K photo
  show the same grain relative to the picture,
- the seed changes every preview frame (moving grain like film projection); the photo uses the seed of
  the frame that was on screen when the shutter was pressed.

The built-in looks are our own interpretations. Fujifilm's simulation names are trademarks and
the original LUTs are not public – hence our own names with "cf. …" in the description.

## Platform roadmap

### macOS (now)
- Rendering via Metal, camera via AVFoundation (nokhwa).
- On first launch macOS asks for camera permission. When started from a terminal, the
  permission applies to the terminal app (or VS Code/iTerm).
- For a real `.app` bundle (later): `cargo bundle` or similar with `NSCameraUsageDescription`
  in the `Info.plist`.

### Linux / Raspberry Pi
- USB cameras work directly via V4L2.
- Pi camera modules (CSI) run through **libcamera**. Short term: `libcamerify filmriss`
  (V4L2 compatibility layer). Long term: a dedicated backend using the `libcamera` crate that
  delivers NV12/YUV420 directly.
- GPU: Pi 4/5 have Vulkan (v3dv), older models OpenGL ES – wgpu falls back automatically.
- Packages needed to build: `libclang-dev libv4l-dev libxkbcommon-dev libwayland-dev`.

### Android (next big step)
The codebase is already prepared (`cargo check -p filmriss-app --target aarch64-linux-android` passes):
- `filmriss-app` is a library with `android_main` (behind `cfg(target_os = "android")`).
- `nokhwa` is not linked on Android (target-specific dependency).

Still to do:
1. `crate-type = ["cdylib", "lib"]` in `filmriss-app` and build with `cargo-apk` or `xbuild`
   (`AndroidManifest`: `android.permission.CAMERA`, `android.hardware.camera`).
2. **`android` camera backend** in `filmriss-camera` using the Camera2 NDK API (`ndk` crate /
   `ndk-sys` with `ACameraManager`, `AImageReader` in `YUV_420_888` format).
3. **YUV straight to the GPU**: upload the Y and UV planes as two textures and convert to RGB
   in the shader – saves the CPU conversion (important on phones; worth it on the Pi too).
4. Runtime permission (JNI → `ActivityCompat.requestPermissions`) and storage via
   `MediaStore` or `Paths::from_root(app.internal_data_path())`.

### Later optimizations
- Separate photo stream at full sensor resolution (the preview stays at ~1080p).
- Render captures on the GPU (readback) instead of the CPU – faster at high resolutions.
- Mipmaps/downsampling for very high-resolution cameras in small windows.
- RAW (DNG) + recipe as metadata → non-destructive development.
