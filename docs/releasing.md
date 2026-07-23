# Releasing Termy

Termy is distributed as one unsigned Windows executable through GitHub Releases. Users download
`termy.exe` directly; no ZIP archive is published. Windows SmartScreen may show an unknown-publisher
warning because the executable does not have a commercial Authenticode certificate.

GitHub Actions generates a Sigstore-backed build-provenance attestation for each executable. A
download can be checked with:

```powershell
gh attestation verify .\termy.exe `
  --repo GitNimay/ADE-agentic-coding-environment
```

## Automatic release flow

Push the intended commit directly to `main`. The `CI` workflow runs formatting, Clippy, and the full
test suite. Only after that push's CI run succeeds, `release.yml`:

1. checks out the exact verified commit;
2. derives a unique semantic version from the workspace major/minor version and CI run number;
3. embeds that release version and builds `ade-app` with the locked dependency graph;
4. renames the output to `termy.exe`;
5. records build provenance; and
6. creates the latest GitHub Release containing only `termy.exe`.

Monitor the runs with:

```powershell
gh run list --branch main
gh run watch --exit-status
```

Official executables use the embedded CI version to check GitHub's latest release metadata in the
background at startup. A newer release opens an in-app notice instead of installing immediately.
**Update and restart** installs that exact release tag and reopens the UI without stopping the
terminal daemon. **Later** dismisses the notice and installs in the background after five minutes
without keyboard or pointer activity; that update takes effect on the next restart. Local builds do
not self-update.
