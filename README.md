# rust-browser

Rust CLI and native GPUI profile manager for persistent Camoufox browser profiles. It uses
[`camoufox-rust`](https://github.com/Pedrvisk/camoufox-rust) 0.9.1 for fingerprint
personas, browser download, launch configuration, and native Juggler browser
control. GeoIP comes from `ipwho.is` through the selected SOCKS5 proxy. No Python
process is used. This project does not read or modify `/opt/workspace/vinted-browser`.

## Commands

```sh
cargo run -- fetch
cargo run -- ui
cargo run -- create account-a --os windows --tab https://www.vinted.fr/
cargo run -- --global-proxy socks5://127.0.0.1:23456 create account-b
cargo run -- create account-c --proxy socks5://127.0.0.1:34567
cargo run -- list
cargo run -- list --json
cargo run -- show account-a
cargo run -- paths account-a
cargo run -- tabs account-a --set https://www.vinted.fr/
cargo run -- doctor
cargo run -- open account-a
cargo run -- open account-a --url https://www.vinted.fr/
cargo run -- scan account-a
cargo run -- refresh account-a
cargo run -- delete account-a
# For non-interactive scripts only:
cargo run -- delete account-a --yes
```

The ordinary app proxy is saved in `browser.sqlite` from the UI's **常规** page;
its initial value is `socks5://127.0.0.1:12334`. CLI commands may override it
temporarily with `RUST_BROWSER_PROXY` or `--global-proxy`. The native UI always
uses the saved setting. Host, port, optional credentials, DNS resolution mode,
and connection timeout are saved together. A profile created without `--proxy` follows the
current saved proxy on every command. `create --proxy` saves an address for
that profile, overriding the global setting. `show` displays the active proxy
and whether the profile follows the global setting. `create`, `open`, `scan`, and `refresh` all verify that the selected
proxy is reachable and query `ipwho.is` through it. A failed lookup stops the
operation; it never falls back to a direct connection.

## Managed proxy rules

Managed proxy rules apply only to profiles using an independent proxy address.
Profiles following the ordinary app proxy have unrestricted concurrency and no
automatic IP switching, even if that address also appears in the managed proxy
catalogue. Rules live in the `managed_proxies` table in `browser.sqlite` under
the Rust data directory. The native proxy editor can also save an optional
username and password there; custom profiles using that address use those
credentials for GeoIP checks and browser connections. Registering a rule does
not rewrite saved profile URLs. `proxy list` reports only whether authentication
is configured and never prints the password.

```sh
cargo run -- proxy set local socks5://127.0.0.1:12334 --name "Local pool" --policy close-previous
cargo run -- proxy set local socks5://127.0.0.1:12334 --name "Local pool" \
  --policy close-previous --switch-url http://127.0.0.1:8080/change-ip \
  --switch-method post --rotate-on-start --wait-seconds 5
cargo run -- proxy list
cargo run -- proxy remove local
```

Policies are `allow-parallel` (default), `reject-new`, and `close-previous`.
`close-previous` requests a normal shutdown of other browser sessions using
that proxy and waits for them to exit before starting the new one. The rule
applies to browsers launched from either the UI or CLI. A proxy-wide launch
lock prevents concurrent starts from bypassing the policy.

When `--rotate-on-start` is enabled, the CLI calls the configured HTTP(S)
endpoint with GET or POST (POST sends an empty body), waits the configured
number of seconds, and checks the exit IP through the SOCKS5 proxy. If the IP
has not changed, it calls the endpoint once more after the first wait and
checks again after another wait. HTTP or GeoIP failures also get at most one
retry. Startup fails if the exit IP remains unchanged or its country, region,
or timezone no longer matches the profile's saved identity. IP switching is
not allowed while another browser using that proxy remains open. The switch
endpoint is called directly. This version accepts an endpoint URL and method;
custom headers and POST bodies are not configured yet.

`create` saves the exit location, including country, region, timezone, locale,
and coordinates. Before `open` or `scan`, the current exit must match the saved
country, region, and timezone. Exit IP, city, and coordinates may change within
that region. Browser fingerprint settings continue to use the saved location.
`refresh` accepts the current exit location and updates the saved location while
keeping the existing fingerprint noise seeds. The browser keeps cookies and site
storage in the profile's own directory. `open` waits until the browser window
closes; Ctrl+C closes it.

`scan` saves BrowserScan text, a viewport screenshot, 24 observed identity
fields, and camoufox-rust's fingerprint verification JSON under `artifacts/`.
It checks the observed proxy IP, timezone, locale, and WebRTC. BrowserScan is a
development diagnostic and does not set or change profile identity. BrowserScan
updates some hashes after initial page load, so `scan` waits for three matching
samples.

The CLI uses Juggler for browser control and proxy setup. Firefox ignores a bare
`--proxy-server` command-line flag, so launching the browser without Juggler would
allow direct traffic. Firefox proxy preferences are also written before startup.

The first `open` or `scan` saves camoufox-rust's complete `PreparedLaunch`,
including Canvas, WebGL, audio, font noise, locale, and timezone. Later launches
reuse it. A different browser version or installation path requires restoring
a compatible browser installation for that pinned identity.
All eight camoufox-rust 0.9.1 crates used by this CLI are pinned in `vendor/`.
`vendor/camoufox` has patches to reuse the saved preparation through Juggler
and apply saved GeoIP configuration; `vendor/camoufox-core` chooses the dominant
country locale; `vendor/camoufox-juggler` cleans up Firefox when startup fails
after spawn. Other crates are currently unchanged upstream copies. Third-party
crates remain Cargo dependencies.
`privacy.baselineFingerprintingProtection=false` prevents Firefox from adding
a new Canvas image salt at every launch; this also removes its per-site Canvas
variation for this profile.

`create --tab` and `tabs --set` accept repeated URLs. `open --url` overrides the
saved tabs for that launch. Without saved tabs, `open` starts at `about:blank`.
Only `scan` navigates to BrowserScan. A new
Profile ID must contain only ASCII letters, digits, `_`, or `-` and start with
a letter or digit. `delete` asks you to type the ID unless `--yes` is used.

## Native profile manager

`cargo run -- ui` opens the GPUI main window using the same data directory and
saved general SOCKS5 proxy as the CLI. The window offers search, proxy and multiple
label filters, profile create/edit/delete, live browser status, and controls to
open or close one, selected, or all running browsers. Labels are stored with each
profile and can be added from its table row. The list renders visible rows and
loads more profiles in batches of 100 while scrolling; there is no pagination
bar. The settings page includes inline autosaved general proxy fields, managed
proxy rules, a reusable label catalogue, and local data paths. Removing or
renaming a catalogue label does not change labels already saved on profiles.
BrowserScan remains a CLI-only development diagnostic.

The new Profile dialog has **Smart** and **Custom** modes. Smart mode detects
the selected proxy's exit through `ipwho.is` and displays the geographic values
that will be saved. Custom mode edits those same fields and can refill them from
GeoIP. In either mode, the selected proxy comes from the current global setting
or this Profile's independent proxy field; the form does not contain a fixed
proxy address. Custom country, region, and timezone must match the actual proxy
exit. WebGL and other fingerprint surfaces remain generated and pinned together.

When the window opens a browser, it starts a separate `open` CLI process and
records output in `logs/<profile-id>.log` under the data directory. A progress
dialog follows the actual proxy check, optional close-previous and IP-switch
policy, GeoIP check, and browser startup. It can continue in the background,
be reopened from the toolbar, or cancel the launch. Launch failures appear in
the dialog. Running status and close requests use a local Unix socket, so the window also sees
browsers started from another CLI process using the same data directory. Closing
the manager window does not close those browsers.

## Native Rust profile API

The library exports `rust_browser::profiles::ProfileService` for a Rust UI to
call in process. It does not start a server. Use the same data directory and
global proxy setting as the CLI:

```rust
use rust_browser::profiles::{
    CreateProfile, ProfileOs, ProfileQuery, ProfileService, ProxyChoice,
    UpdateProfile,
};
use rust_browser::settings::GeneralSettings;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let data_dir = "/path/to/rust-browser-data";
let proxy = GeneralSettings::load(std::path::Path::new(data_dir))?.proxy()?;
let service = ProfileService::with_proxy(data_dir, proxy)?;
let profile = service.create(CreateProfile {
    id: "account-a".into(),
    name: Some("Main account".into()),
    os: ProfileOs::Windows,
    tabs: vec!["https://www.vinted.fr/".into()],
    proxy: ProxyChoice::Global,
    geo: None,
}).await?;
let page = service.list(ProfileQuery {
    search: Some("main".into()), page: 1, page_size: 20,
}).await?;
let updated = service.update(&profile.id, UpdateProfile {
    name: Some("France main".into()), ..Default::default()
}).await?;
let same = service.get(&updated.id).await?;
service.delete(&same.id).await?;
# Ok(())
# }
```

`create` saves GeoIP from `ipwho.is` through the selected SOCKS5 proxy. `get`
returns one profile. `list` searches ID and name without case sensitivity,
sorts by creation time descending then ID ascending, and uses 1-based pages
with `page_size` from 1 to 100. `update` can change name, startup tabs, labels, or
switch between `ProxyChoice::Global` and `ProxyChoice::Custom(url)`. It keeps
the fingerprint and saved location; `open` still checks location drift before
launch. `delete` removes the persona and its browser data directory. Methods
return typed `ProfileError` variants for invalid input, missing or duplicate
profiles, profiles in use, proxy/GeoIP failures, and storage failures.
BrowserScan is not part of this API's profile model.

Data defaults to the executable's directory:
`browser.sqlite` holds profiles, sessions, settings, tags, and managed proxies;
`profiles/<id>/` holds browser user data; `browser/` is the active Camoufox
installation; and `browser-versions/` holds inactive installed versions. The
General settings page can move the browser installation and profile data to
separate storage roots (`browser/` and `browser-versions/` under the browser
root; `profiles/` and `trash/` under the profile root), while SQLite stays
beside the program.
The program directory must be writable by the current user.
`locks/`, `logs/`, and `artifacts/` remain in the program directory; `trash/`
stays beside `profiles/` for profile deletion staging. Runtime
sockets remain in the OS temporary directory. All application paths are defined
in `src/paths.rs`; CLI, GPUI, and library code share this layout. Development
and scripted runs can override the program root with `--data-dir PATH` or
`RUST_BROWSER_DATA_DIR=PATH`. The Rust browser
installation is separate from the Python Camoufox installation.

This CLI does not import the Python project's existing profiles. The scanner
checks 24 observable BrowserScan fields; it cannot prove that every possible
fingerprint surface is identical.

## Validate

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
