# Contributing

Build on Windows with Rust 1.95 or newer, the MSVC build tools, and the Windows
SDK. `rust-version` in `Cargo.toml` is the minimum supported Rust version.

```bash
cargo build --release --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo fmt --check
```

Keep `tempora.exe` and `tempora-recorder.exe` in the same directory when testing the
app. See the [README](README.md) for packaging commands.

## Process boundaries

- The recorder is the sole SQLite writer. It has no application GUI or renderer. Its controls live in a native notification-area menu.
- The dashboard opens SQLite read-only. Send all database writes and settings changes through the recorder's named pipe.

## Where to make changes

The repository is a Cargo workspace of three crates:

| Crate | What it holds |
| --- | --- |
| [`crates/tempora`](crates/tempora/src/lib.rs) | The library: sample-to-interval rules ([observation.rs](crates/tempora/src/observation.rs)), app recognition ([recognition.rs](crates/tempora/src/recognition.rs), display names in [recognition/names.rs](crates/tempora/src/recognition/names.rs)), the database ([storage.rs](crates/tempora/src/storage.rs), reports in [storage/report.rs](crates/tempora/src/storage/report.rs)), app keys and recorder commands ([model.rs](crates/tempora/src/model.rs)), the named pipe ([pipe.rs](crates/tempora/src/pipe.rs)) and safe wrappers over the Windows APIs it observes with ([win.rs](crates/tempora/src/win.rs)). |
| [`crates/tempora-recorder`](crates/tempora-recorder/src/main.rs) | `tempora-recorder.exe`. Control requests and session events are in [host.rs](crates/tempora-recorder/src/host.rs); sampling, exclusions and history writes in [host/capture.rs](crates/tempora-recorder/src/host/capture.rs); the tray icon and menu in [host/tray.rs](crates/tempora-recorder/src/host/tray.rs). |
| [`crates/tempora-dashboard`](crates/tempora-dashboard/src/main.rs) | `tempora.exe`. Page state and refresh scheduling are in [dashboard/mod.rs](crates/tempora-dashboard/src/dashboard/mod.rs), blocking jobs in [dashboard/worker.rs](crates/tempora-dashboard/src/dashboard/worker.rs), views under [dashboard/screens](crates/tempora-dashboard/src/dashboard/screens). |

Some tests need an interactive desktop session (the named pipe, the Windows game list and file version reads) and are marked `#[ignore]`; CI can't run them. Run them locally before changing those areas:

```bash
cargo test --workspace -- --ignored
```

Every `unsafe` block carries a `// SAFETY:` comment saying why it is sound; Clippy enforces this. Prefer adding a safe wrapper to `win.rs` over calling Windows directly.

## Privacy and recognition

- Collect app identity and timing on the PC. Keep diagnostic logs free of app history and identifying details about excluded apps.
- Never collect, store, or log window titles, URLs, keystrokes, screenshots, track names, or call participants. Apply exclusions before writing history.
- Use documented Windows APIs and query-only access to other processes. Keep collection offline and external to games, with no code injection, game memory access, input hooks, overlays, or drivers.
- Recognize apps using evidence already on the user's PC, such as package or executable metadata and local launcher manifests. Avoid hand-maintained executable-name lists. A user edit always wins over automatic recognition.

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE).
