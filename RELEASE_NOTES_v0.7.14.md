## What Changed
- Reworked AI into a right-side resizable sidebar and moved the AI sidebar toggle beside Settings in the top bar.
- Refined sidebar composer controls (attach, agent selector, auto-approve, model picker, stop) and restored context usage control in the header row.
- Increased AI sidebar minimum/default width to keep critical controls (like Stop) visible.
- Improved local provider connection errors with clearer localhost troubleshooting messages.
- CI workflow trigger is manual (`workflow_dispatch`) to prevent automatic CI runs on every push.

## Artifact
- Windows MSI: `Terax_0.7.14_x64_en-US.msi`

## Notes
- MSI is unsigned in this environment (`TAURI_SIGNING_PRIVATE_KEY` is not set).
