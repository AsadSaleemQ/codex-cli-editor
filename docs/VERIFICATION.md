# Verification

This document describes the durable validation contract for Codex CLI Editor. Version-specific fixes and release changes belong in [RELEASE_NOTES.md](../RELEASE_NOTES.md), while CI run details remain in GitHub Actions rather than accumulating here.

## Supported baseline

| Component | Validated baseline |
|---|---|
| Platform | Windows 11 x64 terminals, including the VS Code integrated terminal |
| Enhanced Codex | Codex CLI 0.154.0, upstream tag `rust-v0.154.0` |
| VS Code host | 1.134 and 1.135 |
| Release toolchain | Rust 1.95.0 with Windows MSVC |

Enhanced Codex requires an exact signed compatibility match. An explicit unsupported enhanced request fails visibly; a configured default may fall back to the verified native Codex target. Codex CLI Editor does not discover, shim, launch, validate, or configure another CLI.

## Continuous validation

Every push and pull request must pass:

- repository publication-boundary and credential checks;
- GitHub Actions workflow linting;
- Rust formatting, warnings-denied Clippy, dispatcher tests, and release builds;
- deterministic VS Code extension packaging and bridge behavior tests;
- isolated release-finalizer validation;
- clean and patched pinned-Codex test-suite comparison;
- exact known Windows baseline-failure comparison;
- upstream patch forward/reverse applicability checks;
- pinned Rusty V8 and code-mode-host integrity checks.

The release workflow additionally performs two independent unsigned builds with normalized inputs and timestamps. Artifact hashes must match bit-for-bit before provenance attestations are generated. Signing occurs only after that hosted workflow succeeds and only on the maintainer workstation.

Current results are visible in [GitHub Actions](https://github.com/AsadSaleemQ/codex-cli-editor/actions).

## Local checks

With Rust 1.95 and the Windows MSVC toolchain installed:

```powershell
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked -j 1
.\scripts\check_publication_candidate.ps1
```

The VS Code bridge test can run independently:

```powershell
node .\vscode-extension\extension.test.js
```

## Release acceptance checklist

A publication candidate is not complete until the exact downloadable bundle passes:

- checksum, signed-manifest, artifact-size, and artifact-hash verification;
- clean per-user installation and healthy `status` / `doctor --json` results;
- explicit, defaulted, restored, and literal-argument routes for Codex;
- the complete desktop composer input contract, including mouse placement, drag selection, clipboard text and image paste, undo/redo, scrollback handoff, and prompt-boundary navigation;
- VS Code default-profile and named-profile companion installation;
- fail-closed compatibility behavior for unsupported Codex versions;
- signed update, retained-release rollback, and failure restoration;
- byte-exact PATH restoration when unchanged and preservation of later PATH edits;
- removal of owned state without removing native CLIs or unowned extensions;
- startup-latency checks showing no material launcher penalty;
- SBOM, third-party license, source archive, and provenance inspection.

Only behavior exercised against the final candidate should be marked verified for that release.
