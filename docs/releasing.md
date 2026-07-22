# Releasing Termy

Termy is distributed as a signed MSIX package through GitHub Releases. Users should install
`termy.appinstaller`, not the raw MSIX: Windows then checks the same feed on every launch and in
the background, and installs packages with a higher version automatically.

## One-time setup

1. Obtain a publicly trusted Windows code-signing credential and keep the same certificate subject
   for the lifetime of the package identity. The included workflow supports a password-protected
   PFX. If the certificate authority keeps its key on a hardware token or cloud signing service,
   replace the restore/signing steps with that provider's official CI integration; do not try to
   export a protected key. A self-signed development certificate is not suitable for public
   distribution because every user would have to trust it manually.
2. For a PFX-backed credential, store the PFX and its password as GitHub Actions secrets:

   ```powershell
   [Convert]::ToBase64String([IO.File]::ReadAllBytes("C:\secure\termy-signing.pfx")) |
     gh secret set WINDOWS_SIGNING_CERTIFICATE_BASE64
   gh secret set WINDOWS_SIGNING_CERTIFICATE_PASSWORD
   ```

3. Make the release repository public before distributing the installer. GitHub release downloads
   from a private repository require authentication, which Windows App Installer cannot provide.

Never commit the PFX, its password, or its Base64 representation. The release workflow recreates
the certificate only in the runner's temporary directory and removes it after packaging.

For a new consumer app, Microsoft Store distribution is another strong option: the Store signs
submitted MSIX packages and manages updates. It requires a Partner Center app identity and a
separate submission workflow, so it cannot be configured correctly until that identity exists.

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
re-runs all checks, builds with the locked dependency graph, signs the executable and the MSIX,
verifies the package signature, creates a SHA-256 checksum and update feed, and publishes a GitHub
Release with generated notes.

Monitor the run with:

```powershell
gh run list --workflow release.yml
gh run watch --exit-status
```

Do not reuse a version. Windows compares the four-part MSIX version generated from the Cargo
version (`1.2.3` becomes `1.2.3.0`) and only installs a package with a higher version.
