# Codex CLI Editor v0.2.1

## Compatibility

- Rebases the enhanced desktop composer onto Codex CLI 0.154.0.
- Validates Microsoft VS Code 1.137 while retaining the existing `^1.90.0` extension host range.

# Codex CLI Editor v0.2.0

Codex CLI Editor now uses one identity across every public and technical surface.

## Unified identity

- Renamed the repository, crate, executable, release tags and assets to `codex-cli-editor`.
- Renamed the VS Code extension to `asadsaleemq.codex-cli-editor` version 0.4.0 and its commands to `codexCliEditor.*`.
- Renamed the per-user state directory to `%LOCALAPPDATA%\CodexCLIEditor` and removed the old compatibility command alias.

# Codex CLI Editor v0.1.4

Codex CLI Editor is the final product identity for the enhanced Codex launcher, signed runtime, and VS Code extension.

## Product

- Applies **Codex CLI Editor** consistently across the CLI, VS Code user interface, release titles, documentation, licensing reports, diagnostics, and security statements.
- Supports Codex CLI exclusively. Discovery, shimming, routing, compatibility manifests, diagnostics, and updates are all Codex-specific.
- Keeps the stable `codex-cli-editor` executable, `asadsaleemq.codex-cli-editor` extension ID, `codexCliEditor.*` command IDs, and `AsadSaleemQ/codex-cli-editor` repository slug for safe upgrades.

## Extension 0.3.3

- Presents **Codex CLI Editor** in the Extensions view and Command Palette.
- Restricts prompt navigation and Smart Paste to matching live enhanced-Codex sessions.
- Uses `codexCliEditor.smartPaste` as the Smart Paste command.
- Performs no startup polling, telemetry, or network activity.

## Codex CLI Editor v0.1.3

- Shipped Codex-only discovery, routing, signed compatibility, status, doctor, update, rollback, repair, and uninstall behavior.
- Bundled the enhanced Codex runtime and VS Code extension as one reversible Windows product.
- Restricted VS Code prompt navigation and Smart Paste to matching live enhanced-Codex sessions.

# Codex CLI Editor v0.1.2

Windows x64 patch release of Codex CLI Editor.

## Corrected

- Ctrl+Home and Ctrl+End now always reach the active terminal application instead of falling back to VS Code's scroll-to-top or scroll-to-bottom commands when process matching drifts.
- VS Code bridge tests cover both exact terminal sequences and the no-active-terminal case.

## Included

- Desktop-style Codex composer: touchpad/wheel scrolling, click placement, drag selection, Ctrl+A/C/X/V, Ctrl+Home/End prompt navigation, Shift selection variants, image paste, undo/redo, and a visible mouse cursor.
- Native Codex pass-through with direct `CreateProcessW` launch, console-control forwarding, Job Object cleanup, and exact exit-code propagation.
- Opt-in enhanced invocation plus persistent defaults, status/doctor, signed updates, compatibility fallback, explicit native adoption, and exact PATH rollback on uninstall when Codex CLI Editor owned the unchanged setting, while preserving later edits.

## Compatibility

Validated baseline: Windows 11 x64, VS Code 1.134.0 and 1.135.0, and Codex CLI 0.148.0.

## Integrity

Verify the `.zip` against the adjacent `.sha256` file. The bundle also contains the exact Ed25519-signed compatibility manifest used by the dispatcher.

Codex CLI Editor is an independent modified build and is not affiliated with or endorsed by OpenAI or Microsoft.
