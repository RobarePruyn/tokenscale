# Releasing tokenscale

This document is for the maintainer cutting a new release. End-user install instructions live in [README.md](README.md).

The release pipeline uses [`dist`](https://opensource.axo.dev/cargo-dist/) (formerly `cargo-dist`) to:

- Cross-compile the `tokenscale` binary for macOS (Apple Silicon + Intel), Linux (x86_64 + aarch64), and Windows (x86_64).
- Upload archives + SHA-256 checksums to a GitHub Release.
- Generate three installers — `shell` (`curl | sh`), `powershell` (`iwr | iex`), and `homebrew` (Formula).
- Auto-publish the Homebrew Formula to a separate tap repo so `brew install` works without manual steps.

The whole thing is driven by pushing a Git tag matching the `vMAJOR.MINOR.PATCH` shape. Everything else is automated.

---

## One-time setup (do once, ever)

### 1. Create the Homebrew tap repo

On GitHub, create an **empty public repo** named **`RobarePruyn/homebrew-tokenscale`** (the `homebrew-` prefix is how Homebrew discovers third-party taps). No README, no .gitignore — `dist` writes the Formula directly to `main` on every successful release.

```bash
# Equivalent via the gh CLI:
gh repo create RobarePruyn/homebrew-tokenscale --public --description "Homebrew tap for tokenscale"
```

The tap name in `dist-workspace.toml` (`tap = "RobarePruyn/homebrew-tokenscale"`) must match exactly.

### 2. Create the `HOMEBREW_TAP_TOKEN` repo secret

`dist`'s release workflow needs a personal access token with **write access to the tap repo** to push the Formula. The default `GITHUB_TOKEN` only has permissions on the current repo, not other repos under the same user, so a separate PAT is required.

1. Generate a fine-grained PAT at <https://github.com/settings/personal-access-tokens/new>:
   - **Resource owner**: `RobarePruyn`
   - **Repository access**: "Only select repositories" → `homebrew-tokenscale`
   - **Permissions**: Repository → **Contents: Read and write**
   - Expiration: whatever feels right (we suggest 1 year, calendar a renewal)
2. Copy the token (it's shown once).
3. Add it to the main `tokenscale` repo as a secret:
   - <https://github.com/RobarePruyn/tokenscale/settings/secrets/actions> → **New repository secret**
   - **Name**: `HOMEBREW_TAP_TOKEN`
   - **Secret**: paste the PAT
4. Save.

Without this secret, the `publish-homebrew-formula` job fails with a permissions error and the Formula doesn't get pushed (everything else still works — the GitHub Release with binaries still gets created).

### 3. Create the six macOS notarization secrets (optional but recommended)

The release workflow signs and notarizes macOS binaries with Apple so first-launch on a user's Mac doesn't trigger Gatekeeper's "unidentified developer" block. Requires an Apple Developer Program membership (~$99/yr) and six GitHub Actions secrets.

| Secret | What it is | Where it comes from |
|---|---|---|
| `APPLE_TEAM_ID` | 10-char alphanumeric team ID | developer.apple.com → Membership Details |
| `MACOS_CERTIFICATE` | base64-encoded `.p12` of the Developer ID Application cert | Xcode → Settings → Accounts → Manage Certificates → `+ Developer ID Application`; then export from Keychain Access → `base64 -i cert.p12 -o cert.b64` |
| `MACOS_CERTIFICATE_PASSWORD` | password set when exporting the `.p12` | (you choose during export) |
| `APP_STORE_CONNECT_KEY_ID` | 10-char Key ID for an App Store Connect API key | appstoreconnect.apple.com → Users and Access → Integrations → Team Keys → `+` → role "Developer" |
| `APP_STORE_CONNECT_ISSUER_ID` | UUID at the top of the App Store Connect API key page | (same screen) |
| `APP_STORE_CONNECT_PRIVATE_KEY` | base64-encoded `.p8` private key (one-time download from the same screen) | `base64 -i AuthKey_*.p8 -o key.b64` |

Without these six secrets, the "Sign + notarize macOS binaries" step in the release workflow will fail with a clear error pointing here. The Linux + Windows builds proceed normally; only the macOS build job is affected. If you want to defer notarization, remove (or `if: false`) that step in `.github/workflows/release.yml` — it's a self-contained block annotated with `MANUAL EDIT`.

The signing + notarization step is a manual edit to the dist-generated workflow because cargo-dist 0.31 doesn't have native macOS notarization support ([axodotdev/cargo-dist#1121](https://github.com/axodotdev/cargo-dist/issues/1121)). If you re-run `dist generate --mode=ci` for any reason, the step will be wiped and needs to be re-applied; see the `CUSTOMIZATIONS` block in `dist-workspace.toml` for the inventory.

### 4. Verify locally before the first release

From the repo root:

```bash
# Generates the .github/workflows/release.yml from dist-workspace.toml
# (no-op if you haven't changed any dist config since last time).
dist generate

# Shows what would be built without actually building. Catches misconfig.
dist plan
```

If `dist plan` lists the five expected platform tarballs plus the three installers, you're good.

---

## Cutting a release

### Per-release checklist

1. **Update the version** in `Cargo.toml`'s `[workspace.package]` block (e.g., `version = "0.2.0"`). All workspace members inherit it via `version.workspace = true`.

2. **Update `CHANGELOG.md`** with the release notes (if/when we have one — currently the GitHub Release body is generated from commit messages between tags by `dist`).

3. **Commit + push** to `main`:

   ```bash
   git commit -am "chore: bump to vX.Y.Z"
   git push
   ```

4. **Tag + push the tag**:

   ```bash
   git tag vX.Y.Z
   git push --tags
   ```

5. **Watch CI** at <https://github.com/RobarePruyn/tokenscale/actions> — the `Release` workflow should kick off automatically from the tag push.

The workflow runs in stages:

| Stage | What happens | Typical duration |
|---|---|---|
| `plan` | Parses the version, decides which artifacts to build | ~30 s |
| `build-local-artifacts` (matrix) | Builds frontend (`npm ci && npm run build`) then `cargo build --release` per target | ~5–8 min per matrix entry, parallel |
| `build-global-artifacts` | Generates installers (shell / powershell / homebrew) | ~1–2 min |
| `host` | Uploads everything to a GitHub Release (initially in draft state) | ~30 s |
| `publish-homebrew-formula` | Pushes Formula to `RobarePruyn/homebrew-tokenscale` | ~30 s |
| `announce` | Flips the GitHub Release from draft to published | ~10 s |

End-to-end: **~10–15 minutes** for a five-platform release.

6. **Verify the release** at <https://github.com/RobarePruyn/tokenscale/releases>:
   - Five `.tar.xz` / `.zip` archives, one per target
   - `tokenscale-cli-installer.sh` (shell installer)
   - `tokenscale-cli-installer.ps1` (PowerShell installer)
   - `tokenscale-cli.rb` (Homebrew Formula — also pushed to the tap repo)
   - `sha256.sum`

7. **Smoke-test the installers**:

   ```bash
   # macOS / Linux:
   curl --proto '=https' --tlsv1.2 -LsSf https://github.com/RobarePruyn/tokenscale/releases/download/vX.Y.Z/tokenscale-cli-installer.sh | sh

   # Or via Homebrew (after the tap is published):
   brew tap RobarePruyn/tokenscale
   brew install tokenscale
   tokenscale --version
   ```

If something goes wrong: the release is in your control. `gh release delete vX.Y.Z` removes the GitHub Release, `git tag -d vX.Y.Z && git push --delete origin vX.Y.Z` removes the tag. Then fix and re-tag.

---

## When `dist` config changes

If you ever edit `dist-workspace.toml` or `.github/workflows/build-setup.yml`:

```bash
# Regenerate the release workflow to pick up the changes.
dist generate

# Verify it didn't break the plan.
dist plan
```

Commit both `dist-workspace.toml` (the source of truth) and `.github/workflows/release.yml` (the generated output — yes, we commit it because GitHub Actions needs it on `main`).

If you're bumping `dist` itself:

```bash
cargo install --locked cargo-dist
dist init --yes  # migrates existing config to the new version's schema
dist plan        # verify
```

---

## Why the homebrew tap is a separate repo

Homebrew expects tap repos to be named `homebrew-<tap-name>` and contain only Formulae at the root. Mixing the tap with the main repo would force users to do `brew tap RobarePruyn/tokenscale https://github.com/RobarePruyn/tokenscale.git` and pull the whole tokenscale source tree just to install. Separate tap repo = `brew tap RobarePruyn/tokenscale` works without options, and the tap can be a tiny single-file repo.

`dist` handles the publish flow — we never edit the Formula by hand.

---

## What's NOT in the release pipeline (yet)

These are deferred Phase-3-ish work:

- **APT / DNF / RPM packages**: Linux distro packaging is a community-maintained activity per distro; the prebuilt `.tar.xz` archives are sufficient for users until/unless someone wants to take ownership of a package.
- **Winget manifest**: Windows users on Winget have to install from the PowerShell installer or Scoop bucket (`scoop install tokenscale`) for now. Winget submission is a manual PR per release; would be nice to automate.

### Scoop bucket — semi-automatic

Scoop support exists via a separate hand-maintained bucket repo: <https://github.com/RobarePruyn/scoop-tokenscale>. The manifest there uses Scoop's `autoupdate` block tied to the GitHub Releases URL pattern, so new tokenscale releases propagate to Scoop users on `scoop update` without any maintainer action per release — `dist` doesn't (yet) have native Scoop support, but the autoupdate-driven flow is functionally equivalent for users.

If you need to manually bump the bucket's pinned version (e.g. forcing a hash refresh without an autoupdate trip):

```bash
git clone https://github.com/RobarePruyn/scoop-tokenscale
cd scoop-tokenscale
# Edit bucket/tokenscale.json: bump "version" and refresh "hash"
git commit -am "chore: bump to vX.Y.Z"
git push
```
- **macOS notarization + codesigning**: out of scope for v1. Users will see a "this app is from an unidentified developer" warning on first run; right-click → Open clears it.
- **APT/DNF official-distro inclusion**: requires sustained popularity + a maintainer who can shepherd the package through Debian / Fedora processes. Far future.

If/when these become priorities, see [`docs/decisions.md`](docs/decisions.md) for the rationale for parking them at v1.
