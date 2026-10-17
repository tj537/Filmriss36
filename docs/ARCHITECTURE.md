# Filmriss – Architektur

## Warum Rust + wgpu + 3D-LUTs?

| Anforderung | Lösung |
|---|---|
| Läuft auf macOS, Linux/Raspberry Pi, Windows, später Android | **Rust** kompiliert nativ für alle Ziele, ohne Runtime/GC |
| Flüssige Live-Preview, auch auf schwacher Hardware | **wgpu** (Metal / Vulkan / DX12 / OpenGL ES): die Bildbearbeitung läuft komplett im Fragment-Shader |
| Beliebig komplexe Filmlooks ohne Performance-Einbruch | **3D-LUT als 3D-Textur**: Die Textur-Hardware interpoliert trilinear; ein Look kostet pro Pixel genau *einen* Texturzugriff, egal wie aufwendig er berechnet wurde |
| Eigene Simulationen erstellen | **Rezepte** (TOML) mit verständlichen Reglern, die in eine LUT „gebacken“ werden |
| Fremde Looks nutzen | **`.cube`-Import** (Adobe/Resolve-Standard); Rezepte können auf einer `.cube` aufsetzen |
| Eine UI für Maus und Touch | **egui**: Immediate-Mode-UI, Touch-Events werden wie Pointer behandelt; läuft auf Desktop und Android |

Die Kombination „Rezept → LUT backen → GPU“ ist dieselbe Idee, die Kameras intern nutzen:
die teure Farbmathematik passiert **einmal** (≈1 ms für 33³ Stützstellen), nicht 30× pro Sekunde
für jeden der 2 Mio. Pixel.

## Datenfluss

```
 Kamera-Thread                         UI-Thread (egui)                    GPU
┌──────────────┐  neuestes Bild   ┌───────────────────────┐  write_texture ┌──────────────────────┐
│ nokhwa /     │ ───────────────▶ │ take_frame()          │ ─────────────▶ │ Kamera-Textur RGBA8  │
│ Testbild     │   (Slot, alte    │ Arc<Frame> + ID       │                │                      │
│ YUV→RGBA     │    werden        │                       │  bei Wechsel   │ LUT-Textur 33³ RGBA16F│
└──────────────┘    verworfen)    │ Simulation wählen ────┼──────────────▶ │                      │
        ▲                         │                       │                │ Fragment-Shader:     │
        └── Puffer-Recycling ◀────│ alter Frame frei?     │                │  LUT → Intensität →  │
                                  └───────────────────────┘                │  Vignette → Korn     │
                                             │ Auslöser                    └──────────────────────┘
                                             ▼
                                  Capture-Thread: dieselbe Pipeline auf der CPU (rayon) → JPEG
```

- **Latenz**: Der Kamera-Thread überschreibt einen Ein-Bild-Slot. Kommt die UI nicht hinterher,
  wird nie ein Rückstau aufgebaut.
- **Speicher**: Bildpuffer werden recycelt (Pool), keine 8-MB-Allokation pro Frame.
- **Repaint nur bei Bedarf**: Der Kamera-Thread weckt egui per `request_repaint()`; ohne neue
  Bilder schläft die App.
- **Preview = Foto**: `filmriss_core::process::shade` und `preview.wgsl` implementieren dieselben
  Formeln (inkl. identischem Integer-Hash fürs Korn). Das gespeicherte Foto sieht aus wie die Preview.

## Crates

```
crates/
├── filmriss-core     Plattformunabhängig, ohne GPU/Kamera – voll testbar
│   ├── color.rs      sRGB-Transferkurven, Oklab
│   ├── lut.rs        Lut3D: .cube lesen/schreiben, trilineares Sampling
│   ├── recipe.rs     FilmRecipe: Parameter → bake() → Lut3D
│   ├── presets.rs    eingebaute Rezepte (presets/*.toml, zur Compile-Zeit eingebettet)
│   ├── library.rs    eingebaut + eigene Rezepte + .cube-Dateien, Ordnerverwaltung
│   └── process.rs    CPU-Referenzpipeline (Foto, Thumbnails)
├── filmriss-camera   Capture-Threads, Frame-Slot, Puffer-Pool
│   ├── test_pattern  animiertes Testbild (ColorChecker, Farbrad, Verläufe)
│   └── native        nokhwa: AVFoundation (macOS), V4L2 (Linux/Pi), MSMF (Windows)
└── filmriss-app      egui/eframe-App
    ├── preview.rs    wgpu-Pipeline als egui-Paint-Callback
    ├── preview.wgsl  der Shader
    ├── app.rs        Zustand, Gesten, Layout
    ├── editor.rs     Rezept-Editor
    ├── thumbnails.rs Filmstreifen-Vorschauen
    └── capture.rs    Fotoaufnahme
```

## Das Farbmodell der Rezepte

Reihenfolge in `FilmRecipe::apply` (Eingabe/Ausgabe: sRGB-kodiert, wie Kamerabilder):

1. sRGB → linear; **Belichtung** und **Weißabgleich** (Kanalverstärkung, helligkeitsneutral)
2. **Schwarzweiß** über Kanalmischer (Farbfilter-Simulation: Rot-, Gelb-, Grünfilter …)
3. linear → **Oklab/Oklch** (wahrnehmungsgleichmäßig)
4. **Farbbereiche**: 7 Bänder (Rot, Orange, Gelb, Grün, Cyan, Blau, Magenta) mit weichen
   Übergängen – Farbton, Sättigung, Helligkeit; nahezu neutrale Farben bleiben unberührt
5. globale **Sättigung**
6. **Tonkurve** auf L: Kontrast (Potenz-S-Kurve um Mittelgrau), Lichter, Schatten, Fade;
   die Chroma folgt der Helligkeitsänderung teilweise (wie bei echtem Film)
7. **Split-Toning** für Schatten/Lichter
8. zurück nach sRGB mit **Gamut-Mapping** (Chroma reduzieren statt hart clippen)

Korn und Vignette sind Teil des Rezepts, werden aber *nicht* in die LUT gebacken (sie hängen von
der Pixelposition ab), sondern im Shader berechnet.

Die eingebauten Looks sind eigene Interpretationen. Fujifilms Simulationsnamen sind Marken, die
Original-LUTs nicht öffentlich – deshalb eigene Namen mit „vgl. …“ in der Beschreibung.

## Plattform-Fahrplan

### macOS (jetzt)
- Rendering über Metal, Kamera über AVFoundation (nokhwa).
- Beim ersten Start fragt macOS nach der Kamera-Berechtigung. Aus dem Terminal gestartet, gilt
  die Freigabe für die Terminal-App (bzw. VS Code/iTerm).
- Für ein echtes `.app`-Bundle (später): `cargo bundle` o. Ä. mit `NSCameraUsageDescription`
  in der `Info.plist`.

### Linux / Raspberry Pi
- USB-Kameras funktionieren direkt über V4L2.
- Pi-Kameramodule (CSI) laufen über **libcamera**. Kurzfristig: `libcamerify filmriss`
  (V4L2-Kompatibilitätsschicht). Langfristig: eigenes Backend mit dem `libcamera`-Crate,
  das NV12/YUV420 direkt liefert.
- GPU: Pi 4/5 haben Vulkan (v3dv), ältere Modelle OpenGL ES – wgpu fällt automatisch zurück.
- Benötigte Pakete zum Bauen: `libclang-dev libv4l-dev libxkbcommon-dev libwayland-dev`.

### Android (nächster großer Schritt)
Die Codebasis ist bereits darauf vorbereitet (`cargo check -p filmriss-app --target aarch64-linux-android` läuft durch):
- `filmriss-app` ist eine Bibliothek mit `android_main` (hinter `cfg(target_os = "android")`).
- `nokhwa` wird auf Android nicht gelinkt (target-spezifische Abhängigkeit).

Noch zu tun:
1. `crate-type = ["cdylib", "lib"]` in `filmriss-app` und Build mit `cargo-apk` oder `xbuild`
   (`AndroidManifest`: `android.permission.CAMERA`, `android.hardware.camera`).
2. **Kamera-Backend `android`** in `filmriss-camera` über die Camera2-NDK-API (`ndk`-Crate /
   `ndk-sys` mit `ACameraManager`, `AImageReader` im Format `YUV_420_888`).
3. **YUV direkt auf die GPU**: Y- und UV-Ebene als zwei Texturen hochladen und im Shader nach
   RGB wandeln – spart die CPU-Konvertierung (wichtig auf dem Handy; lohnt sich auch auf dem Pi).
4. Laufzeit-Berechtigung (JNI → `ActivityCompat.requestPermissions`) und Speicherort über
   `MediaStore` bzw. `Paths::from_root(app.internal_data_path())`.

### Spätere Optimierungen
- Separater Foto-Stream in voller Sensorauflösung (Preview bleibt bei ~1080p).
- Aufnahme auf der GPU rendern (Readback) statt CPU – schneller bei hohen Auflösungen.
- Mipmaps/Downsampling für sehr hochauflösende Kameras in kleinen Fenstern.
- RAW (DNG) + Rezept als Metadaten → nicht-destruktives Entwickeln.
