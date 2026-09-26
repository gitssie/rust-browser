# camoufox-rust vendor snapshot

This directory contains the eight `camoufox-rust` crates used by
`rust-browser`, all from the published `0.9.1` crates. `Cargo.toml` uses
`[patch.crates-io]` so direct and transitive references resolve to these local
copies. The crates declare the MPL-2.0 license.

Local changes:

- `camoufox-core/src/locale.rs`: expose deterministic dominant-territory
  locale selection for Profile location pinning.
- `camoufox/src/builder.rs`: allow a Profile to reuse a saved
  `PreparedLaunch` and accept a saved location from ipwho.is, keeping the
  fingerprint configuration stable without substituting a second GeoIP source.
- `camoufox-juggler/src/driver.rs` and `src/transport.rs`: kill and reap Firefox
  if Juggler startup fails after the process is spawned, and kill it on drop on
  Unix.

The other six crates are unchanged `0.9.1` source snapshots. When updating,
update all eight together, reapply the two local patches, run the CLI's
quality checks, and repeat BrowserScan on the same Profile across launches.
