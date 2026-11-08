# Linux release pipeline

1. Fix and test the Linux app. Bump `workspace.package.version` in Cargo.toml
   and the `.deb` name in README.md, then refresh Cargo.lock with
   `CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo update --workspace`.
2. Push the source to `PaulAxer86/entanglo-linux` main.
3. Linux CI builds and tests on Debian 12, creates the `.deb`, and publishes
   a versioned GitHub release with `latest-linux.json`. Existing versions
   are never overwritten: bump the version for every release.
4. Download the release and check the `.deb` checksum and size against
   `latest-linux.json`.
5. In the website repository: set `linuxVersion` in `src/pages/download.astro`,
   add the note to `public/updates/release-notes.json` and
   `src/data/release-notes.json`, copy `latest-linux.json` to `public/updates/`,
   run `npm run build`, then commit and push to main.
6. The Cloudflare Pages Git integration deploys that website commit. Confirm the
   live `updates/latest-linux.json` shows the new version.
7. Linux apps check the HTTPS manifest on startup and hourly. Users can also
   use News & Updates -> Check now. Download/install verifies SHA-256 and size.

The website has no sync workflow: step 5 is done by hand.

For a local build: `cargo test --locked --workspace -- --test-threads=1`,
`cargo build --release --locked -p entanglo-linux`, then
`python3 scripts/package-debian.py --binary target/release/entanglo-linux`.
