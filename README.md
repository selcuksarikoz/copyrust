# copyrust

An interactive file copier for the macOS terminal, including NTFS and ExFAT external disks.

## Why it's fast

- **One indexing pass.** The target folder is read exactly once into an in-memory index; later conflict lookups (`overwrite`, `skip`, `rename`) never touch the slow disk again. Finder, by contrast, may rescan the whole volume.
- **No GUI overhead.** Plain `fs::copy` straight from the terminal, no SpotlightCoordinator/UI churn.
- **Disk-aware scheduling.** The target volume's filesystem and HDD/SSD type are detected automatically. HDDs use one copy worker; SSDs use four unless several sources map to the same destination.
- **Filesystem-aware conflicts.** NTFS and ExFAT names are matched without regard to letter case. `Skip`, `Overwrite`, and `Rename (keep both)` use the indexed destination paths.

## Install & run

Rust compiles to a native binary — build once, then the executable runs anywhere without a Rust toolchain.

```sh
cargo build --release   # one-time optimized build
```

**Option 1 — build once, install to PATH (recommended)**

```sh
cp target/release/copyrust /usr/local/bin/   # or sudo, depending on your setup
copyrust                                     # runs from anywhere
```

**Option 2 — `cargo install` (global, managed by cargo)**

```sh
cargo install --path .
copyrust    # installed into ~/.cargo/bin
```

**Option 3 — `cargo run` (during development)**

```sh
cargo run --release
```

**Option 4 — plain `rustc` (works, but not recommended)**

```sh
rustc -O --edition 2024 src/main.rs -o copyrust
```

This skips Cargo's `lto`/optimize settings from `Cargo.toml`, so the binary is slower than a `cargo build --release` one.

Requires a Rust toolchain (`rustup`) — edition 2024 needs Rust 1.85+.

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

Source folders (paste/drop folder paths): /Volumes/Data/photos
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

Edition 2024, std-only (no external crates) — the binary has zero runtime dependencies.

```sh
cargo build --release
```

## License

MIT
