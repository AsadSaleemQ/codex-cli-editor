# Codex CLI Editor

Codex CLI Editor turns Codex terminals into chat-style editors with smart clipboard paste and familiar shortcuts.

Codex CLI Editor combines the VS Code extension in this package with an enhanced Codex CLI build from the [Codex CLI Editor GitHub repository](https://github.com/AsadSaleemQ/codex-cli-editor). The extension activates its prompt shortcuts and Smart Paste behavior only for a matching live enhanced-Codex session. Every other terminal keeps normal VS Code behavior.

## Keyboard shortcuts

| Shortcut | Behavior | Scope |
|---|---|---|
| `Ctrl+A` | Select the complete editable prompt. | Enhanced Codex |
| `Home` / `End` | Move to the start / end of the current prompt line. | Enhanced Codex |
| `Ctrl+Home` / `Ctrl+End` | Move to the start / end of the complete prompt. | Enhanced Codex via this extension |
| `Shift+Left` / `Shift+Right` | Extend or shrink the selection by one character. | Enhanced Codex |
| `Shift+Up` / `Shift+Down` | Extend the selection across lines. | Enhanced Codex |
| `Shift+Home` / `Shift+End` | Select to the start / end of the current line. | Enhanced Codex |
| `Ctrl+Shift+Left` / `Ctrl+Shift+Right` | Extend the selection by word. | Enhanced Codex |
| `Ctrl+Shift+Home` / `Ctrl+Shift+End` | Select to the start / end of the complete prompt. | Enhanced Codex |
| `Ctrl+C` | Copy selected prompt text; without a selection, preserve interrupt/cancel. | Enhanced Codex |
| `Ctrl+X` | Cut the selected prompt text. | Enhanced Codex |
| `Ctrl+V` | Use VS Code text paste when clipboard text is present; otherwise forward `Ctrl+V` so Codex can handle image paste. | Enhanced Codex via this extension |
| `Ctrl+Alt+V` | Force clipboard-image paste. | Enhanced Codex |
| `Ctrl+Z` | Undo the last prompt edit. | Enhanced Codex |
| `Ctrl+Shift+Z` | Redo the last undone prompt edit. | Enhanced Codex |
| `Backspace` / `Delete` | Delete the complete selected range. | Enhanced Codex |

## Mouse and scrolling

| Input | Behavior |
|---|---|
| Click | Place the prompt cursor. |
| Click and drag | Select prompt text across lines. |
| Double-click / triple-click | Select a word / line. |
| Mouse wheel / touchpad | Hand scrolling back to VS Code terminal history; the next edit restores prompt interaction. |

## Extension runtime

| Item | Runtime behavior |
|---|---|
| Public commands | `codexCliEditor.promptHome`, `codexCliEditor.promptEnd`, `codexCliEditor.smartPaste` |
| Activation | Lazy `onCommand` activation when one of the four registered command IDs is invoked—normally by `Ctrl+Home`, `Ctrl+End`, or `Ctrl+V` while a terminal has focus. |
| Prompt navigation | Sends fixed xterm Ctrl+Home / Ctrl+End sequences only when a matching live enhanced-Codex prompt owns input; otherwise uses VS Code's terminal-history commands. |
| Smart Paste | Runs only for a matching live enhanced-Codex session. Non-empty text uses VS Code terminal paste; otherwise the extension forwards `Ctrl+V` for Codex image paste. Other terminals use normal VS Code paste. |
| Background activity | None. No startup activation, polling, status-bar process, telemetry, or network request. |

## Compatibility

| Layer | Supported or validated baseline |
|---|---|
| Operating system | Windows 11 x64 validated. The complete Codex CLI Editor product is not currently supported on macOS or Linux. |
| Editor | Microsoft VS Code 1.134, 1.135, and 1.137 validated; the manifest accepts VS Code `^1.90.0`. VS Code forks are untested. |
| Enhanced Codex | Codex CLI 0.154.0 (`rust-v0.154.0`) with an exact signed compatibility match. This provides the full chat-style composer. |
| Terminal | VS Code integrated terminal on the validated Windows baseline. Other terminal hosts do not load this extension. |

Validated combinations are Codex CLI Editor 0.2.1 and 0.2.2 with native Codex 0.154.0 on VS Code 1.134.0, 1.135.0, and 1.137.0, plus Codex CLI Editor 0.2.0 with native Codex 0.148.0 on VS Code 1.134.0 and 1.135.0. Live Windows terminal acceptance was completed on VS Code 1.137.0 for Codex 0.154.0; the other listed combinations are automated compatibility baselines.

## Install the complete product

Download the latest Windows release from [GitHub Releases](https://github.com/AsadSaleemQ/codex-cli-editor/releases). The signed bundle installs the enhanced Codex build, launcher, and this extension as one reversible product.
