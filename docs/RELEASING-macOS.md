# Notarizing the macOS demo zoo

The release already ships **single-file macOS binaries** (the payload is linked
in as a `__DATA,__socrateszoo` Mach-O section — see `docs/ARCHITECTURE.md`). By
default they are **ad-hoc signed**: they run locally, but a *downloaded* copy
trips Gatekeeper and needs it cleared once —
`xattr -d com.apple.quarantine ./<demo>`.

To make downloads open with **no warning**, sign them with a Developer ID
certificate and notarize them with Apple. The release workflow already has the
step wired (`.github/workflows/release.yml` → "Developer ID sign + notarize");
it stays dormant until the six repo secrets below exist. This is the one-time
setup.

## Prerequisite

An **Apple Developer Program** membership, identity-verified (individual or, if
you enrolled with a DUNS, organization). Nothing here works until enrollment is
complete.

## 1. Create a Developer ID Application certificate

In Xcode: **Settings → Accounts → (your Apple ID) → Manage Certificates → +
→ Developer ID Application**. (Or the Apple Developer portal → Certificates →
+ → Developer ID Application.) This is the certificate that signs software for
distribution outside the App Store.

## 2. Export it as a `.p12`

- **Keychain Access → My Certificates**, find `Developer ID Application: NAME
  (TEAMID)`, right-click → **Export…** → `.p12`, and set an export password.
- Grab the exact identity string (you'll need it for `MACOS_SIGN_IDENTITY`):
  ```sh
  security find-identity -v -p codesigning
  # → "Developer ID Application: Your Name (TEAMID)"
  ```
- Base64 the `.p12`:
  ```sh
  base64 -i DeveloperID.p12 | pbcopy   # → MACOS_CERT_P12_BASE64
  ```

## 3. Create an App Store Connect API key (for `notarytool`)

- **App Store Connect → Users and Access → Integrations → App Store Connect
  API → +**. Give it **Developer** access. Download the `.p8` (you can only
  download it once).
- Note the **Key ID** and the **Issuer ID** shown on that page.
  ```sh
  base64 -i AuthKey_XXXXXXXXXX.p8 | pbcopy   # → MACOS_NOTARY_KEY_P8_BASE64
  ```

## 4. Set the six repository secrets

GitHub → the repo → **Settings → Secrets and variables → Actions → New
repository secret**:

| Secret | Value |
|--------|-------|
| `MACOS_CERT_P12_BASE64` | base64 of the Developer ID `.p12` |
| `MACOS_CERT_PASSWORD` | the `.p12` export password |
| `MACOS_SIGN_IDENTITY` | `Developer ID Application: NAME (TEAMID)` |
| `MACOS_NOTARY_KEY_P8_BASE64` | base64 of the App Store Connect `.p8` |
| `MACOS_NOTARY_KEY_ID` | the API key's Key ID |
| `MACOS_NOTARY_ISSUER_ID` | the API key's Issuer ID (a UUID) |

## 5. Cut the release

**Actions → Release → Run workflow**, tag `v0.7.0`. The macOS leg now
re-signs every zoo binary with Developer ID (hardened runtime + secure
timestamp) and notarizes them in one `notarytool` submission. The run
refreshes the existing `v0.7.0` draft's assets — then publish it from the
Releases page.

> If a *second* `v0.7.0` draft appears instead of the existing one being
> updated, delete the older draft and keep the freshly-built one.

## Notes

- **Stapling.** A bare command-line binary can't have a notarization ticket
  stapled to it (`stapler` only handles `.app`/`.dmg`/`.pkg`), so Gatekeeper
  verifies online on first run — one network round-trip, then it's cached. For
  fully-offline "just works", ship the zoo as a stapled `.dmg`; that's a later
  enhancement, not required for notarization to take effect.
- **Rotation.** The `.p8` API key and the Developer ID cert expire; regenerate
  and re-set the secrets when they do.
