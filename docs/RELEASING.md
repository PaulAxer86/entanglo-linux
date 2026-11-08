# Linux release pipeline

1. Fix and test the Linux app. Bump `workspace.package.version` in Cargo.toml.
2. Push the source to `PaulAxer86/entanglo-linux` main.
3. Linux CI builds and tests on Debian 13, creates the `.deb`, and publishes
   a versioned GitHub release with `latest-linux.json`. Existing versions
   are never overwritten: bump the version for every release.
4. The website's Linux sync workflow checks for a new release every 15 minutes,
   verifies the package checksum, builds the website, then commits the package
   and `public/updates/latest-linux.json` to the website main branch.
5. The existing Cloudflare Pages Git integration deploys that website commit.
6. Linux apps check the HTTPS manifest on startup and hourly. Users can also
   use News & Updates -> Check now. Download/install verifies SHA-256 and size.

The workflows use only their own repository's GITHUB_TOKEN; no cross-repository
personal token is required. Automatic publication can also be triggered with
Run workflow from the website Actions page.

For a local build: `cargo test --locked --workspace -- --test-threads=1`,
`cargo build --release --locked -p entanglo-linux`, then
`python3 scripts/package-debian.py --binary target/release/entanglo-linux`.
