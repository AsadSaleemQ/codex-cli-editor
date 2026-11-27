# Codex CLI Editor technical guide

## What ships

The Git repository contains the Rust dispatcher, a compact patch against a pinned Codex commit, compatibility schemas, the VS Code companion source, documentation, and release automation. Upstream source trees, build directories, local validation output, and compiled release binaries remain outside version control. Published Windows artifacts belong in GitHub Releases.

A release bundle contains:

- `codex-cli-editor.exe`: installer, dispatcher, compatibility guard, updater, doctor, and uninstaller.
- `codex-enhanced.exe`: the pinned Codex build with the desktop composer patch.
- `codex-code-mode-host.exe`: the matching upstream code-mode helper.
- `codex-cli-editor.vsix`: the Codex CLI Editor extension, which connects VS Code terminal input to the chat-style composer and provides smart text/image paste and prompt-navigation bindings.
- `compatibility-manifest.json` and `.sig`: Ed25519-signed artifact and compatibility metadata.
- `THIRD_PARTY_LICENSES_CODEX_CLI_EDITOR.html` and `THIRD_PARTY_LICENSES_CODEX.html`: generated dependency license texts for the two Rust binary sets.

## Commands

```text
codex-cli-editor install [--dry-run]
codex-cli-editor status
codex-cli-editor doctor [--json]
codex-cli-editor default codex
codex-cli-editor restore codex
codex-cli-editor update --bundle DIRECTORY
codex-cli-editor rollback [--release RELEASE]
codex-cli-editor repair --adopt-native codex
codex-cli-editor uninstall
codex-cli-editor run codex -- ARGS...
codex codex-cli-editor [-- CODEX_ARGS...]
```

Installation adds one owned shim directory to the beginning of the current user's PATH and broadcasts the Windows environment change. When VS Code is safely discovered beside its official CLI launcher, installation adds the bundled Codex CLI Editor extension and records ownership so uninstall removes only an extension Codex CLI Editor added. VS Code profiles maintain separate extension and keybinding registries, so the extension and its bindings must exist in the profile that owns the terminal workspace. A default request fails without changing state when native Codex was not discovered; after installing Codex, `codex-cli-editor repair --adopt-native codex` adds its route.

Before changing PATH, Codex CLI Editor records the exact raw registry value, including its type and expansion text. If PATH is unchanged, uninstall restores that snapshot byte-for-byte. If PATH changed later, uninstall removes only Codex CLI Editor's owned entry and preserves the later edits. A pre-existing shim entry is never claimed or removed. Cleanup is confined to the owned `%LOCALAPPDATA%\CodexCLIEditor` tree and never traverses a reparse-point directory. Self-uninstall removes its running shim from command resolution before completing cleanup and reports any inert Windows-locked residue that must be removed after exit.

## Codex behavior

The VS Code extension owns only the terminal-level delivery layer. It sends standard xterm modified-key sequences for Ctrl+Home and Ctrl+End so the enhanced Codex composer receives complete-prompt navigation instead of VS Code scrollback commands. Its Ctrl+V handler delegates non-empty text to VS Code's terminal paste command and sends terminal-native Ctrl+V when clipboard text is empty so image paste can reach the CLI. It does not read prompt or transcript content, store clipboard content, or transmit it. User-level keybindings have final precedence, and named profiles keep independent extension and keybinding registries.

Enhanced Codex is selected only by an explicit `codex codex-cli-editor` invocation or an enabled Codex default. A signed cached manifest must support the exact native Codex version. Invalid signatures, rollback sequences, unsupported Codex versions, or expired manifests cannot authorize an enhanced binary. An unlisted VS Code host version produces a visible warning and continues because host drift does not change the pinned Codex binary. A defaulted route otherwise degrades to verified native Codex; an explicit enhanced request fails visibly.

A legitimate in-place native update may self-adopt only when the recorded canonical path, package root, expected vendor/package family, and executable shape remain unchanged. Cold native probes have a bounded 60-second budget; after a timeout, an identity-validated default Codex route warns and forwards natively, while explicit enhanced Codex still fails visibly. Codex CLI Editor records completed adoptions with old/new versions and hashes in a bounded journal. Relocation or identity changes require explicit `repair --adopt-native codex`.

## Updates and rollback

`update --bundle` verifies the detached Ed25519 signature, monotonic manifest sequence, expiry, minimum dispatcher version, Codex-only compatibility metadata, and every artifact's declared size and SHA-256. It stages and smoke-tests enhanced Codex with a 60-second cold-artifact budget before acquiring the state lock. Activation retains the active release and two prior signed releases, replaces cache/shims transactionally, and restores prior files if state publication fails. `codex-cli-editor rollback` re-verifies a retained Codex-only signed release and activates it without lowering the highest observed manifest sequence. Restarting reloads an installed update but does not install source-tree changes.

A dispatcher-changing update must be launched from the new external bundle. This prevents a Windows executable from overwriting itself. Existing sessions retain their already-open executable; locked shims cause update failure and rollback rather than mixed state.

## Build

Requirements: Git, Rust 1.99.1, and the Windows MSVC toolchain. The local GNU/LLVM toolchain can build the patched Codex but Rusty V8 does not publish the matching code-mode-host archive; the release workflow therefore uses `x86_64-pc-windows-msvc`.

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test -j 1 --offline
```

To pass a literal first argument named `codex-cli-editor` to native Codex, use `codex -- codex-cli-editor`.

The optional `--` immediately after the `codex-cli-editor` control token is a consumed separator. To forward a literal leading delimiter, repeat it, for example `codex codex-cli-editor -- -- help`.

To reproduce the upstream patch manually:

```powershell
git clone https://github.com/openai/codex.git upstream-codex
git -C upstream-codex checkout 3d86e43b075adeba2193ec5c22ac92f55afd82e6
git -C upstream-codex apply ..\codex-cli-editor\patches\codex\rust-v0.172.0\0001-desktop-composer.patch
cargo build --manifest-path upstream-codex\codex-rs\Cargo.toml --release -j 1 -p codex-cli -p codex-code-mode-host
```

Release dispatches are serialized, and preparation requires the requested manifest sequence to exceed every existing release or draft manifest sequence. GitHub performs two unsigned builds with normalized paths and deterministic timestamps, refuses the candidate unless every artifact hash matches bit-for-bit, and provenance-attests the unsigned executables, VSIX, source archive, and SBOM. The Ed25519 seed corresponding to `compatibility/public-key.hex` remains only on the maintainer workstation. `scripts/publish_draft_release.ps1` accepts a successful release-workflow run ID, requires that run to match local `HEAD`, downloads only its primary candidate, runs the isolated finalizer locally, checks the generated public key, publishes a signed draft, and deletes its temporary `.artifacts` workspace in `finally`. `src/bin/sign_release.rs` signs exact manifest bytes. The signed draft must then pass downloaded-artifact install, bare default and explicit routes for both CLIs, native restoration, uninstall, residue, exact raw-PATH, and latency checks before publication. v0.1 does not include Authenticode signing or established SmartScreen reputation.

## Documentation

- [Verification and current release gates](docs/VERIFICATION.md)
- [Desktop composer behavior](docs/DESKTOP_COMPOSER_BEHAVIOR.md)
- [Updates and rollback](docs/UPDATE_AND_ROLLBACK.md)
- [Security policy](SECURITY.md)
- [Contributing](CONTRIBUTING.md)
- [Release notes](RELEASE_NOTES.md)
- [Third-party notices](THIRD_PARTY_NOTICES.md)

## License and trademarks

Codex CLI Editor source is Apache-2.0. The patched Codex files retain upstream licensing and statements of modification. OpenAI, Codex, Microsoft, and VS Code are trademarks of their respective owners. Their names identify compatibility only.
