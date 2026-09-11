# Codex CLI Editor

**Turn Codex CLI terminals into chat-style editors with smart clipboard paste and familiar shortcuts.**

Click to place the cursor, select with the mouse or keyboard, copy and cut, paste text or images, undo and redo, and navigate long drafts like a desktop chat composer.

One repository ships the complete Codex-only product: an enhanced Codex build, the `codex-cli-editor` launcher, and the `asadsaleemq.codex-cli-editor` VS Code extension. Codex CLI Editor supports Codex CLI exclusively and does not manage any other command-line assistant.

**Codex CLI Editor** is the product and technical identity everywhere. Machine-readable locations use the equivalent slug or identifier forms: `codex-cli-editor`, `asadsaleemq.codex-cli-editor`, `codexCliEditor.*`, and `%LOCALAPPDATA%\CodexCLIEditor`.

## Chat-style controls

### Keyboard shortcuts

| Shortcut | Behavior | Scope |
|---|---|---|
| `Ctrl+A` | Select the complete editable prompt. | Enhanced Codex |
| `Home` / `End` | Move to the start / end of the current prompt line. | Enhanced Codex |
| `Ctrl+Home` / `Ctrl+End` | Move to the start / end of the complete prompt. | Enhanced Codex via the extension |
| `Shift+Left` / `Shift+Right` | Extend or shrink the selection by one character. | Enhanced Codex |
| `Shift+Up` / `Shift+Down` | Extend the selection across lines. | Enhanced Codex |
| `Shift+Home` / `Shift+End` | Select to the start / end of the current line. | Enhanced Codex |
| `Ctrl+Shift+Left` / `Ctrl+Shift+Right` | Extend the selection by word. | Enhanced Codex |
| `Ctrl+Shift+Home` / `Ctrl+Shift+End` | Select to the start / end of the complete prompt. | Enhanced Codex |
| `Ctrl+C` | Copy selected prompt text; without a selection, preserve interrupt/cancel. | Enhanced Codex |
| `Ctrl+X` | Cut the selected prompt text. | Enhanced Codex |
| `Ctrl+V` | Use VS Code text paste when clipboard text is present; otherwise forward `Ctrl+V` so Codex can handle image paste. | Enhanced Codex via the extension |
| `Ctrl+Alt+V` | Force clipboard-image paste. | Enhanced Codex |
| `Ctrl+Z` | Undo the last prompt edit. | Enhanced Codex |
| `Ctrl+Shift+Z` | Redo the last undone prompt edit. | Enhanced Codex |
| `Backspace` / `Delete` | Delete the complete selected range. | Enhanced Codex |

### Mouse and scrolling

| Input | Behavior |
|---|---|
| Click | Place the prompt cursor. |
| Click and drag | Select prompt text across lines. |
| Double-click / triple-click | Select a word / line. |
| Mouse wheel / touchpad | Hand scrolling back to VS Code terminal history; the next edit restores prompt interaction. |
| Completed response selection | Select and copy terminal output normally when the prompt is empty. |

## Capabilities

### Launcher and CLI integration

- Run enhanced Codex explicitly with `codex codex-cli-editor` or make it the default.
- Forward arguments, console-control events, exit codes, and process cleanup without shell-wrapper ambiguity.
- Inspect installation health with human-readable or JSON diagnostics.
- Adopt legitimate in-place native CLI updates while rejecting relocation or identity drift.

### Safe lifecycle management

- Verify enhanced artifacts and compatibility metadata with an embedded Ed25519 public key.
- Fail visibly for unsupported explicit enhanced requests and fall back to verified native Codex for default routes.
- Stage updates transactionally, retain signed prior releases, and support verified rollback.
- Restore native defaults at any time.
- Uninstall only owned files and PATH entries while preserving unrelated edits and pre-existing extensions.

See the [desktop composer guide](docs/DESKTOP_COMPOSER_BEHAVIOR.md) for the complete input contract.

## Install

Download the latest Windows x64 ZIP and adjacent `.sha256` file from [GitHub Releases](https://github.com/AsadSaleemQ/codex-cli-editor/releases), verify the checksum, extract the archive, and run:

```powershell
.\codex-cli-editor.exe install
```

Codex CLI Editor installs its per-user launcher and, when VS Code is available, the bundled **Codex CLI Editor** extension. Reload VS Code after installation. Named VS Code profiles must enable the extension in the profile used by the terminal.

Open a new terminal so it inherits the updated user PATH, then try:

```powershell
codex codex-cli-editor
codex-cli-editor status
codex-cli-editor doctor
```

The source repository intentionally excludes compiled release executables. Cloning the repository is for development, not installation.

## Choose how Codex launches

| Goal | Command |
|---|---|
| One enhanced Codex session | `codex codex-cli-editor` |
| Enhanced Codex by default | `codex-cli-editor default codex` |
| Restore native Codex | `codex-cli-editor restore codex` |
| Pass `codex-cli-editor` literally to native Codex | `codex -- codex-cli-editor` |

## Manage the installation

```text
codex-cli-editor status
codex-cli-editor doctor [--json]
codex-cli-editor update --bundle DIRECTORY
codex-cli-editor rollback [--release RELEASE]
codex-cli-editor repair --adopt-native codex
codex-cli-editor uninstall
```

Updates are explicit and bundle-based; startup never blocks on a network download. See the [update and rollback contract](docs/UPDATE_AND_ROLLBACK.md) for recovery and ownership behavior.

## Compatibility and trust

| Layer | Supported or validated baseline |
|---|---|
| Operating system | Windows 11 x64. macOS and Linux are not currently supported. |
| Editor | Microsoft VS Code 1.134, 1.135, and 1.137 validated; the extension manifest accepts VS Code `^1.90.0`. VS Code forks are untested. |
| Enhanced Codex | Codex CLI 0.154.0 (`rust-v0.154.0`) with an exact signed compatibility match. |
| Terminal | VS Code integrated terminal on the validated Windows baseline. |
| Release toolchain | Rust 1.95.0 with Windows MSVC. |

Host-version drift may warn without changing the pinned enhanced binary; suspicious native-target or artifact changes fail closed.

Release assets are reproducibly built, hash-checked, provenance-attested, and finalized with a signing key that is never uploaded to GitHub. Windows may still show SmartScreen on an unsigned executable without established reputation.

## Documentation

- [Desktop composer behavior](docs/DESKTOP_COMPOSER_BEHAVIOR.md) — complete mouse, keyboard, clipboard, selection, and scrolling contract.
- [Technical guide](TECHNICAL_GUIDE.md) — architecture, process fidelity, compatibility, building, and release design.
- [Update and rollback](docs/UPDATE_AND_ROLLBACK.md) — update verification, retention, rollback, uninstall, and failure recovery.
- [Verification](docs/VERIFICATION.md) — supported baseline, automated gates, and acceptance checklist.
- [Security policy](SECURITY.md) — trust boundaries and vulnerability reporting.
- [Release notes](RELEASE_NOTES.md) — version-specific changes only.

## Project boundary

Codex CLI Editor is an independent modified build and is not affiliated with or endorsed by OpenAI or Microsoft. Codex source is used under Apache-2.0.
