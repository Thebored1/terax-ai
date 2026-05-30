## What Changed
- Fixed AI workspace root resolution to follow active terminal CWD, so snapshots track the directory you actually `cd` into.
- This resolves stale startup-path behavior (for example staying on `C:/Users/Paper/Downloads`).

## Artifact
- Windows MSI: `Terax_0.7.13_x64_en-US.msi`

## Notes
- MSI is unsigned in this environment (`TAURI_SIGNING_PRIVATE_KEY` is not set).
