# Filmriss 🎞️

Eine schnelle, offene Kamera-App mit **Live-Filmsimulationen** – geschrieben in Rust.
Läuft auf dem Mac, unter Linux (auch Raspberry Pi) und Windows; Android ist vorbereitet.

- **Live-Preview auf der GPU** (wgpu: Metal/Vulkan/DX12/GL) – Filmlooks als 3D-LUT, ein Texturzugriff pro Pixel
- **15 eingebaute Simulationen**, inspiriert von klassischen Filmen (Dia, Negativ, Kino, Schwarzweiß …)
- **Eigene Simulationen**: Rezept-Editor mit Live-Vorschau, gespeichert als lesbares TOML
- **`.cube`-LUTs importieren** (einfach in den LUT-Ordner legen) und **exportieren** (für Lightroom, Resolve & Co.)
- **Korn & Vignette** wie beim Film, identisch in Preview und Foto
- **Touch und Maus**: Wischen zum Wechseln, Halten für Vorher/Nachher, große Bedienelemente

> Status: frühe Entwicklungsversion. Getestet wird zunächst auf macOS.

## Schnellstart (macOS)

```bash
# 1. Rust installieren (falls noch nicht vorhanden)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 2. Starten
git clone https://github.com/tj537/filmriss36.git
cd filmriss36
cargo run            # Debug-Build (Abhängigkeiten werden trotzdem optimiert → flüssig)
cargo run --release  # maximale Performance
```

Beim ersten Start fragt macOS nach dem **Kamerazugriff** – die Freigabe gilt für die App, aus der du
`cargo run` startest (Terminal, iTerm, VS Code …). Falls versehentlich abgelehnt:
*Systemeinstellungen › Datenschutz & Sicherheit › Kamera*. Ohne Kamera läuft ein animiertes Testbild.

## Bedienung

| Aktion | Maus / Trackpad | Touch | Tastatur |
|---|---|---|---|
| Simulation wechseln | Filmstreifen klicken, ziehen, Zwei-Finger-Wisch | Wischen | ← / → |
| Foto aufnehmen | Auslöser | Auslöser | Leertaste |
| Original zeigen | „Vorher“ halten / Preview gedrückt halten | gedrückt halten | C halten |
| Geteilte Ansicht | „◑ Vergleich“ | „◑ Vergleich“ | S |
| UI ausblenden | Doppelklick | Doppeltippen | H |
| Technische Infos (FPS, Format, GPU) | „ℹ Info“ | | I |

Fotos landen in `~/Pictures/Filmriss/` als JPEG (optional zusätzlich das Original).

## Eigene Filmsimulationen

**Im Editor:** Tab *Rezept* öffnen, Regler verschieben (wirkt sofort live), Namen eingeben,
*„Als neue Simulation“* speichern. Doppelklick auf einen Regler setzt ihn zurück.

**Als Datei:** Rezepte sind TOML-Dateien im Rezept-Ordner
(`~/Library/Application Support/de.Filmriss.Filmriss/recipes/`). Alle Felder sind optional:

```toml
name = "Mein Kodak-Abend"
description = "Warm, weich, leicht verblasst"
temperature = 0.12
contrast = 0.1
fade = 0.1
saturation = 0.1
grain = 0.25

[hues.orange]          # Hauttöne
lightness = 0.1

[highlight_tone]       # Split-Toning der Lichter
hue = 70.0             # Oklch-Farbton in Grad (≈ 70 = Bernstein, 200 = Cyan)
strength = 0.3
```

Die eingebauten Rezepte unter [`crates/filmriss-core/presets/`](crates/filmriss-core/presets) sind gute Vorlagen.

**`.cube`-LUTs:** in den LUT-Ordner (`…/de.Filmriss.Filmriss/luts/`) kopieren und
*„Bibliothek neu laden“* klicken. Jede LUT erscheint als eigene Simulation und lässt sich im
Editor feintunen – das Ergebnis wird als Rezept mit Verweis auf die LUT gespeichert.

## Technik

Rust · egui/eframe · wgpu · nokhwa · Oklab-Farbraum · 3D-LUTs (33³, RGBA16F)

Details zu Pipeline, Farbmodell und dem Weg zu Android & Raspberry Pi: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)

```
crates/filmriss-core     Farbmathematik, LUTs, Rezepte, Presets (ohne GPU/Kamera, getestet)
crates/filmriss-camera   Kamera-Threads, Testbild, native Backends
crates/filmriss-app      egui-App, wgpu-Shader, Editor, Aufnahme
```

```bash
cargo test -p filmriss-core   # Tests der Farbpipeline
cargo clippy --workspace
```

## Lizenz

MIT oder Apache-2.0, nach Wahl.
