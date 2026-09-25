# rust-browser

Independent Rust CLI for persistent Camoufox browser profiles. It uses
[`camoufox-rust`](https://github.com/Pedrvisk/camoufox-rust) 0.9.1 for fingerprint
personas, browser download, GeoIP configuration, SOCKS5 proxy configuration,
and native Juggler browser control. No Python process is used. This project does not
read or modify `/opt/workspace/vinted-browser`.

## Commands

```sh
cargo run -- fetch
cargo run -- create account-a --os windows --tab https://www.vinted.fr/
cargo run -- list
cargo run -- list --json
cargo run -- show account-a
cargo run -- paths account-a
cargo run -- tabs account-a --set https://www.vinted.fr/ --set https://www.browserscan.net/
cargo run -- doctor
cargo run -- open account-a
cargo run -- open account-a --url https://www.browserscan.net/
cargo run -- scan account-a
cargo run -- refresh account-a
cargo run -- delete account-a
# For non-interactive scripts only:
cargo run -- delete account-a --yes
```

`open` and `scan` always use `socks5://127.0.0.1:12334` and block WebRTC. They check that
the local proxy endpoint is reachable, then ask camoufox-rust to resolve GeoIP
through that proxy. If the proxy or GeoIP lookup fails, launch fails. The browser
keeps cookies and site storage in the profile's own directory. The saved persona
keeps its full prepared fingerprint configuration across launches. `open` waits until the
browser window closes; Ctrl+C closes it. `scan` saves BrowserScan text, a viewport
screenshot, and camoufox-rust's fingerprint verification JSON under `artifacts/`.
It verifies the proxy IP, timezone, disabled WebRTC, and 24 BrowserScan identity
fields against a baseline saved on the first scan. BrowserScan updates some
hashes after initial page load, so `scan` waits for three matching samples.

The CLI uses Juggler for browser control and proxy setup. Firefox ignores a bare
`--proxy-server` command-line flag, so launching the browser without Juggler would
allow direct traffic. Firefox proxy preferences are also written before startup.

The first `open` or `scan` saves camoufox-rust's complete `PreparedLaunch`,
including Canvas, WebGL, audio, font noise, locale and timezone. Later launches
reuse it and reject a changed proxy exit IP or browser version. `refresh`
explicitly rotates the launch identity for the current proxy exit and clears
the BrowserScan baseline; run `scan` again afterward. This preserves cookies
and site storage, so use it only when you intend to change the identity.
All eight camoufox-rust 0.9.1 crates used by this CLI are pinned in `vendor/`.
`vendor/camoufox` has a small patch to reuse the saved preparation through
Juggler; `vendor/camoufox-juggler` cleans up Firefox when startup fails after
spawn. Other crates are currently unchanged upstream copies. Third-party crates
remain Cargo dependencies.
`privacy.baselineFingerprintingProtection=false` prevents Firefox from adding
a new Canvas image salt at every launch; this also removes its per-site Canvas
variation for this profile.

`create --tab` and `tabs --set` accept repeated URLs. `open --url` overrides the
saved tabs for that launch. Without saved tabs, `open` uses BrowserScan. A new
Profile ID must contain only ASCII letters, digits, `_`, or `-` and start with
a letter or digit. `delete` asks you to type the ID unless `--yes` is used.

Data lives in the platform application data directory under `rust-browser`:
`browser/` for the Camoufox installation, `personas/` for the upstream FileStore,
and `profiles/<id>/` for browser data. Override this root with
`--data-dir PATH` or `RUST_BROWSER_DATA_DIR=PATH`. The Rust browser installation
is separate from the Python Camoufox installation.

This CLI does not import the Python project's existing profiles. The scanner
checks 24 observable BrowserScan fields; it cannot prove that every possible
fingerprint surface is identical.

## Validate

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
