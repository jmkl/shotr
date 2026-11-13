# shotr

A lightweight Windows screenshot tool: press **PrintScreen**, drag to select a
region, and it is copied to the clipboard and saved as WebP. Hold **Shift**
while releasing the mouse to open the annotation editor instead.

## Features

- **PrintScreen hotkey** (global, event-driven, no polling).
- **Frozen full-desktop capture** with a dimmed selection overlay; works across
  multiple monitors.
- **Clipboard copy** as a native `CF_DIB` (paste into any app).
- **WebP export** (quality 90) saved to `Pictures\Screenshots`.
- **Annotation editor** (opened with Shift):
  - Tools: Pen, Rectangle, Filled rectangle (redaction), Arrow, Text.
  - 7 colors including white, 3 stroke widths.
  - Undo / Redo, smooth stabilized pen strokes.
  - Live preview, then a single render on **Done**.
- **System tray** icon with Quit.
- **Per-user install** via `install.nu` (no admin rights, starts at login).

## Requirements

- Windows 10/11.
- Rust stable (MSVC toolchain).
- (Optional) [Nushell](https://www.nushell.sh/) for `install.nu`.

## Build

```sh
cargo build --release
```

The binary is at `target\release\shotr.exe`.

## Run

```sh
target\release\shotr.exe
```

A tray icon appears. Press **PrintScreen**, drag a region, release.

- Normal release: copy + save.
- Release with **Shift** held: open the annotation editor.

In the editor: pick a tool/color/width, draw, then **OK** (or Enter) to finish
or **Cancel** (or Esc) to discard.

## Install (autostart)

```nu
nu install.nu
```

Copies the built exe to `%LOCALAPPDATA%\Programs\shotr` and adds a per-user
Run entry so it starts at login. Use `nu install.nu --uninstall` to remove.

## Project layout

```
src/
  main.rs        entry point
  app.rs         lifecycle, message loop, finish pipeline
  capture.rs     virtual-desktop grab + crop
  selection.rs   selection overlay
  annotation.rs  annotation editor
  smooth.rs      stabilized pen strokes
  clipboard.rs   CF_DIB clipboard copy
  image.rs       pixel buffer + WebP encoding
  storage.rs     known-folder save path
  tray.rs        tray icon + menu
  hotkey.rs      global PrintScreen hotkey
```

## License

See repository.