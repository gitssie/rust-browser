# Browser schema fixture

`camoufox-152-properties.json` is the `properties.json` shipped with the local
Camoufox 152.0.4 beta.31 installation. It lets the French profile lifecycle test
exercise real launch preparation and schema validation without a browser install,
network requests, or launching a browser process. The executable and add-on in
that test are directory fixtures only.

`identity-probe.js` reads browser-visible locale, screen, font metrics, Canvas,
audio samples, and WebGL from a local blank page. The ignored
`french_live_identity_survives_browser_restart` test launches two separate
browser processes with one persistent test profile and compares these values.
Run it with `RUST_BROWSER_LIVE_INSTALL` set to an installed Camoufox directory;
it does not use a user's existing browsing profile or visit a remote test site.
