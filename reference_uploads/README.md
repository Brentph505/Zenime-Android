# Fixed-files reference bundle guide

This folder is for the original Zenime Android fix bundle and its review notes.

## Handling rules

- Treat `Zenime-Android-fixed-files.zip` as a reference bundle, not as instructions to overwrite the current checkout.
- Do not extract it over the working tree or apply `zenime-build-fix.patch` automatically.
- Review the supplied files and compare each changed file against the current `main` branch before choosing any changes.
- This guide commit does not alter the app source, Gradle files, Rust files, or CI workflow.

## Original uploaded inputs

- ZIP: `Zenime-Android-fixed-files.zip`
  - SHA-256: `acb355dcfcbaa803798f52d8b2f76ec3053a1fb841a27b857a4414f48b6b1d93`
  - Contains 18 archive entries, including `src/lib.rs`, `src/providers.rs`, `Cargo.toml`, `Cargo.lock`, `.github/workflows/android-rust-apk.yml`, `android/app/build.gradle.kts`, and `LocalHlsProxy.kt`.
- Patch: `zenime-build-fix.patch`
  - SHA-256: `827920cf98f2a014ebb1d36dccc7eed0e1d100e1ca165d4b91dabea679bb5137`

The hashes are included so the original attachments can be checked for accidental changes. The uploaded ZIP and patch should remain unchanged when transferred into the repository.
