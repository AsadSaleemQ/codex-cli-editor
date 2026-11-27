# Third-party notices

Codex CLI Editor includes or builds upon third-party software under their respective licenses.

## OpenAI Codex

The enhanced Codex artifact is a modified build of `openai/codex`, pinned to tag `rust-v0.172.0` and commit `3d86e43b075adeba2193ec5c22ac92f55afd82e6`. Upstream is licensed under Apache License 2.0. The upstream `LICENSE` and `NOTICE` files are reproduced at the repository root and must accompany binary distributions.

Codex CLI Editor's changes are represented by `patches/codex/rust-v0.172.0/0001-desktop-composer.patch`. The patch extends upstream composer selection with desktop keyboard shortcuts, inline mouse handling, clipboard cut, VS Code bridge integration, and regression tests.

## Rust dependencies

Rust dependency names, versions, sources, checksums, and license metadata are fixed by `Cargo.lock` files and generated build metadata. Release automation retains those lockfiles, emits a CycloneDX SBOM, and packages generated full dependency-license reports for both Codex CLI Editor and the modified Codex binaries.

All product and company names identify compatibility only and remain trademarks of their respective owners.
