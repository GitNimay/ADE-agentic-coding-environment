# Releasing Termy

Termy is currently distributed as a free, unsigned portable Windows ZIP through GitHub Releases.
Users extract the archive and run `termy.exe`. Windows SmartScreen may show an unknown-publisher
warning because the executable does not have a commercial Authenticode certificate.

Every release also includes a SHA-256 checksum. GitHub Actions generates a Sigstore-backed build
provenance attestation for the ZIP in the public transparency log. A downloaded archive can be
checked with:

```powershell
Get-FileHash .\termy-windows-x64.zip -Algorithm SHA256
gh attestation verify .\termy-windows-x64.zip `
  --repo GitNimay/ADE-agentic-coding-environment
```

## Publish a version

1. Update `workspace.package.version` in `Cargo.toml` using `MAJOR.MINOR.PATCH` notation.
2. Run the full checks and commit the resulting `Cargo.lock` update:

   ```powershell
   cargo fmt --all --check
   cargo clippy --workspace --all-targets --locked -- -D warnings
   cargo test --workspace --locked
   ```

3. After the version commit is on `main`, create and push a matching annotated tag:

   ```powershell
   git tag -a v0.2.0 -m "Termy 0.2.0"
   git push origin v0.2.0
   ```

The tag starts `.github/workflows/release.yml`. It verifies the tag and Cargo versions match,
re-runs all checks, builds with the locked dependency graph, creates the portable archive and
checksum, records build provenance, and publishes a GitHub Release with generated notes.

Monitor the run with:

```powershell
gh run list --workflow release.yml
gh run watch --exit-status
```

Do not reuse or move a published version tag. Users update manually by replacing `termy.exe` with
the copy from a newer release; their workspace database remains in the local application-data
directory.
