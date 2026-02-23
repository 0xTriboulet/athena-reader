# Athena Reader

Athena Reader is a native desktop speed-reading tool:

- Paste a screenshot from your clipboard
- Run OCR locally (offline after models are cached)
- Stream the extracted text at a target WPM using an ORP-style centered word display

This repository is a Rust workspace with:

- `athena-core`: OCR + text normalization/tokenization + reading session logic
- `athena-app`: egui/eframe GUI application

<p align="center">
  <img src="Screencast%20from%202026-02-22%2019-33-17.gif" alt="Demo" />
</p>

## How it works (User flow)

1. Launch the app.
2. Bring text into the app:
   - **Import**: pick an image file, a PDF, or a `.txt` file.
   - **Paste Image**: paste an image from the system clipboard (button or **Ctrl/Cmd+V**).
3. Processing runs in a background thread (UI stays responsive).
   - OCR from images typically takes a few seconds depending on the image size/quality and your machine.
4. The bottom preview shows the first **25 words** of extracted text.
5. Double-click the preview to open **Edit Text** in a separate window; **Save** updates the text used for playback.
6. The reading view shows the first word/chunk immediately; press **Play** (or **Space**) to begin streaming.

## Controls / Shortcuts

- Paste image: **Ctrl/Cmd+V**
- Play/Pause: **Space**
- Step back/forward: **Left / Right arrow**
- Restart (resets to first word and pauses): **R**

## Importing PDFs

PDF import extracts embedded text (for text-based PDFs). Scanned/image-only PDFs will generally produce empty text and show an error.

## UI Components (Quick Guide)

- **Top controls**
  - **Import**: select an image or PDF.
  - **Paste Image**: reads an image from the clipboard and runs OCR.
  - **Play / Pause / Prev / Next / Restart**: playback controls.
  - Sliders:
    - **WPM**: 100–900
    - **Chunk**: 1–5 words per step
    - **Font**: 18–200
  - **Theme**: Light / Dark / High Contrast
- **Preview panel**
  - Shows the first 25 words of extracted text.
  - Double-click to open **Edit Text**; **Save** rebuilds the reading session from the edited text (paused).
- **Bottom-right status**
  - Shows the current status plus a processing timer/spinner while OCR/PDF extraction is running.

## OCR backend and models

The default OCR backend is:

- `oar-ocr` (ONNX models executed via ONNX Runtime)

Default model artifacts are downloaded from Hugging Face (MIT licensed):

- Repo: `GetcharZp/go-ocr`
- Files:
  - `paddle_weights/det.onnx`
  - `paddle_weights/rec.onnx`
  - `paddle_weights/dict.txt`

Models are cached locally in the OS cache directory (via `directories::ProjectDirs`), under:

`~/.cache/athena-reader/ocrs/` (Linux default)

`%LOCALAPPDATA%\\athena\\reader\\cache\\ocrs\\` (Windows default)

## Environment variables

### Model paths (local files)

- `ATHENA_OCR_DETECTION_MODEL`
- `ATHENA_OCR_RECOGNITION_MODEL`
- `ATHENA_OCR_DICT_PATH`

### Model downloads (override URLs and optional integrity pins)

- `ATHENA_OCR_DETECTION_URL` / `ATHENA_OCR_DETECTION_SHA256`
- `ATHENA_OCR_RECOGNITION_URL` / `ATHENA_OCR_RECOGNITION_SHA256`
- `ATHENA_OCR_DICT_URL` / `ATHENA_OCR_DICT_SHA256`

### Cache directory + download toggle

- `ATHENA_OCR_MODEL_CACHE_DIR` (override the directory where models are stored)
- `ATHENA_OCR_DISABLE_DOWNLOAD` (set to `1`, `true`, `yes`, or `on` to prevent downloading)

## Build and run

### Prerequisites

- Rust toolchain (stable) + Cargo

> Note: `oar-ocr` uses ONNX Runtime and may download native runtime binaries depending on its configuration/features.

### Run the app

```bash
cargo run -p athena-app
```

### Build

```bash
cargo build --release -p athena-app
```

## Testing / Linting

```bash
cargo fmt
cargo clippy --workspace -- -D warnings
cargo test --workspace
```
