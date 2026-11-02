# copyrust

An interactive file copier for the macOS terminal, including NTFS and ExFAT external disks.

## Why it's fast

- **One indexing pass.** The target folder is read exactly once into an in-memory index; later conflict lookups (`overwrite`, `skip`, `rename`) never touch the slow disk again. Finder, by contrast, may rescan the whole volume.
- **No GUI overhead.** Plain `fs::copy` straight from the terminal, no SpotlightCoordinator/UI churn.
- **Disk-aware scheduling.** The target volume's filesystem and HDD/SSD type are detected automatically. HDDs use one copy worker; SSDs use four unless several sources map to the same destination.
- **Filesystem-aware conflicts.** NTFS and ExFAT names are matched without regard to letter case. `Skip`, `Overwrite`, and `Rename (keep both)` use the indexed destination paths.

## Install & run

Rust compiles both commands to native binaries. The copier runs without extra tools; the optimizer calls `ffmpeg`, `ffprobe`, `cwebp`, and `avifenc`.

```sh
cargo build --release   # one-time optimized build
```

**Option 1 — build once, install to PATH (recommended)**

```sh
cp target/release/copyrust target/release/optimizer /usr/local/bin/   # or sudo, depending on your setup
copyrust   # or optimizer; both run from anywhere
```

**Option 2 — `cargo install` (global, managed by cargo)**

```sh
cargo install --path .
copyrust    # installed into ~/.cargo/bin
```

**Option 3 — `cargo run` (during development)**

```sh
cargo run --release --bin copyrust
```

Requires a Rust toolchain (`rustup`) — edition 2024 needs Rust 1.85+.

## Image optimizer

Run `target/release/optimizer` (or `./optimizer` when copied to the project root). It asks for one or more source folders, lists the available JPG/JPEG, PNG, WebP, and AVIF extensions with counts, and lets you select several at once. Then choose the output format, quality from 1 to 100 (default 80), and whether to delete originals (default Yes).

The optimizer processes up to two images at once on an HDD or four on an SSD. Progress output is refreshed at most ten times per second.

AppleDouble `._*` files, the `.system` directory, and interrupted conversion temp files are skipped. Ordinary names beginning with `_` remain selectable.

The converted image is placed beside its source with the new extension. Existing files are kept: the optimizer adds ` (1)`, ` (2)`, etc. to avoid a name collision. When converting to the same format and keeping the original, it adds ` optimized` to the new name. Output dimensions are checked against the source before replacing or deleting anything. Failed conversions leave the original in place.

The result shows total input and output bytes for successful conversions. With original deletion enabled, it also shows estimated space saved (or extra space used if the output grew). When originals are kept, it reports the additional space occupied by the new files.

Both `copyrust` and `optimizer` return to the source-folder prompt after each job. Enter `q` at that prompt to exit.

PNG remains lossless; its quality number controls compression effort and file size, not visual quality. Animated images are not supported. Some metadata and color profiles may not survive conversion between formats; JPEG also cannot retain transparency.

## Usage

To run the wizard directly without installing, use `target/release/copyrust`:

```sh
target/release/copyrust   # or just ./main (copied to the project root)
```

The wizard walks you through:

1. **Source folders** (one or more). The wizard lists their available extensions with file counts for multi-selection.
2. **Target folder** (created if missing). Its filesystem and HDD/SSD type are detected automatically.
3. Whether to **preserve subfolder structure** or flatten everything into the target root.
4. **Conflict action** when a file already exists: `Overwrite`, `Skip`, or `Rename (keep both)`.
5. **Post-copy action**: delete from source (default) or keep it.

### File selection

Choose several extension numbers at once, such as `1,3-5`. Press Enter or type `all`
to select every extension, including files without one. The matching files are selected before
the target options appear.

A live progress bar shows percent / files done / failures. When finished you get a summary (`copied`, `renamed`, `skipped`, `source deleted`, `failed`).

Paths can be pasted from Finder ("Copy as Pathname" — surrounding quotes are stripped).

## Example session

```
=== copyrust ===

Source folders (paste/drop folder paths, q = quit): /Volumes/Data/photos
2841 file(s) in source. Available extensions:
  1. .jpg — 2400 file(s)
  2. .png — 100 file(s)
  3. .webp — 341 file(s)

Choose extensions (e.g. 1,3-5; empty = all): 1,2

2500 file(s) selected.

Target folder (created if missing): /Volumes/BackupNTFS/photos
Target disk: ntfs / HDD (1 worker(s)).
...
[============================  ]  91% (2275/2500) failed: 0
```

## Notes

- Delete-after-copy happens **only after** the copy itself succeeded; on shutdown, Ctrl-C or transfer errors the source file is kept.
- Rename strategy is `name (1).ext`, `name (2).ext`, ... so nothing is ever silently lost.
- If the volume is also indexed by Spotlight, add it to *System Settings → Siri & Spotlight → Spotlight Privacy* — the tool avoids re-reading the target, but Spotlight churn on the volume still slows the drive.

## Build

Edition 2024, std-only (no external crates). The optimizer requires the external image tools listed above.

```sh
cargo build --release
```

## License

MIT
