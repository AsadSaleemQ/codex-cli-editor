use std::cell::Cell;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use crate::CliKind;
use crate::compatibility::{
    CompatibilityManifest, Freshness, VerifiedManifest, release_key_is_development, verify_manifest,
};
use crate::discovery::{
    DiscoveryOptions, discover_native, probe_release_version, refresh_recorded_target,
    same_package_identity, sha256_file,
};
use crate::error::{CodexCliEditorError, Result};
use crate::registry::{
    prepend_shim, read_user_path, remove_shim, restore_user_path, write_user_path,
};
use crate::state::{
    ManifestCacheRecord, ReleaseRecord, State, StateStore, ensure_not_reparse, replace_file,
};
use crate::version::normalized_version;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) fn install(dry_run: bool) -> Result<()> {
    let store = StateStore::for_current_user()?;
    let current_executable = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(|source| CodexCliEditorError::io("current executable", source))?;
    if let Some(existing) = store.load()?
        && is_installed_codex_cli_editor_shim(&existing, &current_executable)
    {
        println!(
            "Codex CLI Editor is already installed; run the extracted release copy to revalidate or update the installation."
        );
        return Ok(());
    }
    let shim_directory = store.root().join("shims");
    let discovery = DiscoveryOptions::from_environment(shim_directory.clone())?;
    let mut native_targets = BTreeMap::new();
    match discover_native(&discovery) {
        Ok(target) => {
            native_targets.insert(CliKind::Codex, target);
        }
        Err(error) => {
            eprintln!("warning: native Codex discovery failed safely: {error}");
        }
    }
    if native_targets.is_empty() {
        return Err(CodexCliEditorError::NoSupportedCliFound);
    }

    let source_dispatcher = current_executable;
    let source_directory = source_dispatcher
        .parent()
        .ok_or_else(|| CodexCliEditorError::UnsafeTarget(source_dispatcher.clone()))?;
    let enhanced_source = source_directory.join("codex-enhanced.exe");
    let helper_source = source_directory.join("codex-code-mode-host.exe");
    let manifest_source = source_directory.join("compatibility-manifest.json");
    let signature_source = source_directory.join("compatibility-manifest.sig");
    let vscode_extension_source = source_directory.join("codex-cli-editor.vsix");
    let verified = verify_release_bundle(
        source_directory,
        &source_dispatcher,
        &enhanced_source,
        &helper_source,
        &manifest_source,
        &signature_source,
        0,
    )?;
    if !vscode_extension_source.is_file() {
        return Err(CodexCliEditorError::MissingReleaseArtifact(
            vscode_extension_source,
        ));
    }
    verify_declared_artifact(&verified, "codex-cli-editor.vsix", &vscode_extension_source)?;
    if let Some(codex) = native_targets.get(&CliKind::Codex) {
        let version = normalized_version(&codex.version);
        if !verified.manifest.supports_codex(version) {
            eprintln!(
                "warning: native Codex {version} is not validated for enhanced mode; installation will keep native Codex as the default"
            );
        }
    }
    let version_directory = store
        .root()
        .join("versions")
        .join(release_directory_name(verified.manifest.sequence));

    if dry_run {
        println!("Codex CLI Editor install dry run");
        println!("  state: {}", store.root().display());
        println!("  shims: {}", shim_directory.display());
        println!("  release: {}", version_directory.display());
        for (kind, target) in native_targets {
            println!("  native {}: {}", kind.as_str(), target.path.display());
        }
        return Ok(());
    }

    let vscode_extension = crate::vscode::install(&vscode_extension_source)?;
    if vscode_extension == crate::vscode::InstallOutcome::Unavailable {
        eprintln!(
            "warning: VS Code was not discovered; the Codex CLI Editor extension was not installed"
        );
    }
    let vscode_extension_added = vscode_extension == crate::vscode::InstallOutcome::Added;
    let rollback_path = RefCell::new(None);
    let new_install_started = Cell::new(false);
    let existing_manifest_sequence = Cell::new(0);
    let result = store.transaction(|current| {
        if let Some(mut existing) = current {
            existing_manifest_sequence.set(existing.highest_manifest_sequence);
            existing.vscode_extension_added |= vscode_extension_added;
            return Ok((existing, false));
        }
        new_install_started.set(true);

        std::fs::create_dir_all(&version_directory)
            .map_err(|source| CodexCliEditorError::io(&version_directory, source))?;
        std::fs::create_dir_all(&shim_directory)
            .map_err(|source| CodexCliEditorError::io(&shim_directory, source))?;
        let installed_dispatcher = version_directory.join("codex-cli-editor.exe");
        let enhanced_target = version_directory.join("codex.exe");
        let helper_target = version_directory.join("codex-code-mode-host.exe");
        atomic_copy(&source_dispatcher, &installed_dispatcher)?;
        atomic_copy(&enhanced_source, &enhanced_target)?;
        atomic_copy(&helper_source, &helper_target)?;
        atomic_copy(
            &manifest_source,
            &version_directory.join("compatibility-manifest.json"),
        )?;
        atomic_copy(
            &signature_source,
            &version_directory.join("compatibility-manifest.sig"),
        )?;
        verify_declared_artifact(&verified, "codex-cli-editor.exe", &installed_dispatcher)?;
        verify_declared_artifact(&verified, "codex-enhanced.exe", &enhanced_target)?;
        verify_declared_artifact(&verified, "codex-code-mode-host.exe", &helper_target)?;
        let enhanced_version = probe_release_version(&enhanced_target)?;
        let enhanced_version = normalized_version(&enhanced_version).to_owned();
        if !verified.manifest.supports_codex(&enhanced_version) {
            return Err(CodexCliEditorError::UnsupportedCodexVersion(
                enhanced_version,
            ));
        }
        let compatibility_directory = store.root().join("compatibility");
        let manifest_target = compatibility_directory.join("manifest.json");
        let signature_target = compatibility_directory.join("manifest.sig");
        atomic_copy(&manifest_source, &manifest_target)?;
        atomic_copy(&signature_source, &signature_target)?;
        atomic_copy(
            &installed_dispatcher,
            &shim_directory.join("codex-cli-editor.exe"),
        )?;
        if native_targets.contains_key(&CliKind::Codex) {
            atomic_copy(&installed_dispatcher, &shim_directory.join("codex.exe"))?;
        }

        let path_snapshot = read_user_path()?;
        let next_path = prepend_shim(&path_snapshot, &shim_directory)?;
        let path_changed = next_path != path_snapshot.data;
        if path_changed {
            rollback_path.replace(Some(path_snapshot.clone()));
            write_user_path(path_snapshot.value_type, &next_path)?;
        }

        let mut state = State::new(VERSION);
        state.pre_install_user_path = Some(path_snapshot);
        state.shim_directory = Some(shim_directory.clone());
        state.path_entry_added = path_changed;
        state.vscode_extension_added = vscode_extension_added;
        state.native_targets = native_targets;
        state.highest_manifest_sequence = verified.manifest.sequence;
        state.manifest_cache = Some(ManifestCacheRecord {
            manifest_path: manifest_target,
            signature_path: signature_target,
            sequence: verified.manifest.sequence,
            expires_unix: verified.manifest.expires_unix,
        });

        let enhanced_artifact = verified
            .manifest
            .artifact("codex-enhanced.exe")
            .ok_or_else(|| CodexCliEditorError::ArtifactNotDeclared("codex-enhanced.exe".into()))?;
        let (_, modified_unix_ms) = artifact_metadata(&enhanced_target)?;
        state.active_release = Some(ReleaseRecord {
            version: release_directory_name(verified.manifest.sequence),
            directory: version_directory.clone(),
            codex_version: enhanced_version,
            sha256: enhanced_artifact.sha256.clone(),
            file_size: enhanced_artifact.size,
            modified_unix_ms,
        });
        Ok((state, true))
    });

    if result.is_err() {
        let _ = crate::vscode::uninstall_if_owned(vscode_extension_added);
        if let Some(snapshot) = rollback_path.into_inner() {
            let _ = restore_user_path(&snapshot);
        }
        if new_install_started.get() {
            let _ = cleanup_owned_root(store.root());
        }
    }
    if result? {
        println!("Codex CLI Editor installed successfully.");
        if vscode_extension != crate::vscode::InstallOutcome::Unavailable {
            println!("  reload VS Code once to activate chat-style terminal editing");
        }
        println!("  shims: {}", shim_directory.display());
        println!(
            "  run `codex-cli-editor doctor` in a new PowerShell terminal; it reports if a machine-scope command outranks the per-user shim"
        );
    } else if newer_bundle_available(verified.manifest.sequence, existing_manifest_sequence.get()) {
        println!(
            "Codex CLI Editor verified a newer release bundle but did not activate it; run `codex-cli-editor update --bundle \"{}\"`.",
            source_directory.display()
        );
    } else {
        println!(
            "Codex CLI Editor is already installed; existing state was revalidated and republished."
        );
    }
    Ok(())
}

fn newer_bundle_available(candidate_sequence: u64, installed_sequence: u64) -> bool {
    candidate_sequence > installed_sequence
}

fn is_installed_codex_cli_editor_shim(state: &State, current_executable: &Path) -> bool {
    let Some(shims) = state.shim_directory.as_ref() else {
        return false;
    };
    let Ok(shims) = shims.canonicalize() else {
        return false;
    };
    current_executable.parent() == Some(shims.as_path())
        && current_executable
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("codex-cli-editor.exe"))
}

struct ReleaseBundle {
    dispatcher: PathBuf,
    enhanced: PathBuf,
    helper: PathBuf,
    vscode_extension: PathBuf,
    manifest: PathBuf,
    signature: PathBuf,
}

impl ReleaseBundle {
    fn from_directory(directory: &Path) -> Result<Self> {
        let directory = directory
            .canonicalize()
            .map_err(|source| CodexCliEditorError::io(directory, source))?;
        Ok(Self {
            dispatcher: directory.join("codex-cli-editor.exe"),
            enhanced: directory.join("codex-enhanced.exe"),
            helper: directory.join("codex-code-mode-host.exe"),
            vscode_extension: directory.join("codex-cli-editor.vsix"),
            manifest: directory.join("compatibility-manifest.json"),
            signature: directory.join("compatibility-manifest.sig"),
        })
    }

    fn directory(&self) -> &Path {
        self.dispatcher.parent().expect("bundle has a parent")
    }
}

pub(crate) fn update(bundle_directory: &Path) -> Result<()> {
    let store = StateStore::for_current_user()?;
    let current_executable = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(|source| CodexCliEditorError::io("current executable", source))?;
    update_with_store(
        bundle_directory,
        &store,
        &current_executable,
        probe_release_version,
    )
}

fn update_with_store<F>(
    bundle_directory: &Path,
    store: &StateStore,
    current_executable: &Path,
    smoke_probe: F,
) -> Result<()>
where
    F: Fn(&Path) -> Result<String>,
{
    let prepared = store.load()?.ok_or(CodexCliEditorError::NotInstalled)?;
    let bundle = ReleaseBundle::from_directory(bundle_directory)?;
    let verified = verify_release_bundle(
        bundle.directory(),
        &bundle.dispatcher,
        &bundle.enhanced,
        &bundle.helper,
        &bundle.manifest,
        &bundle.signature,
        prepared.highest_manifest_sequence,
    )?;
    verify_declared_artifact(&verified, "codex-cli-editor.vsix", &bundle.vscode_extension)?;
    if verified.manifest.sequence <= prepared.highest_manifest_sequence {
        return Err(CodexCliEditorError::NoUpdateAvailable);
    }

    verify_managed_codex_compatibility(&prepared, &verified.manifest)?;

    let installed_dispatcher = prepared
        .active_release
        .as_ref()
        .ok_or(CodexCliEditorError::EnhancedUnavailable)?
        .directory
        .join("codex-cli-editor.exe");
    let dispatcher_changed =
        sha256_file(&installed_dispatcher)? != sha256_file(&bundle.dispatcher)?;
    let current_executable = current_executable
        .canonicalize()
        .map_err(|source| CodexCliEditorError::io(current_executable, source))?;
    let owned_root = store
        .root()
        .canonicalize()
        .map_err(|source| CodexCliEditorError::io(store.root(), source))?;
    if dispatcher_changed && current_executable.starts_with(&owned_root) {
        return Err(CodexCliEditorError::ExternalUpdaterRequired);
    }

    let release_name = release_directory_name(verified.manifest.sequence);
    let versions_directory = store.root().join("versions");
    std::fs::create_dir_all(&versions_directory)
        .map_err(|source| CodexCliEditorError::io(&versions_directory, source))?;
    let unique = format!(
        "{}.staging.{}.{:032x}",
        release_name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let staging = versions_directory.join(unique);
    let final_directory = versions_directory.join(&release_name);
    if final_directory.exists() {
        quarantine_interrupted_release(&versions_directory, &final_directory)?;
    }
    std::fs::create_dir(&staging).map_err(|source| CodexCliEditorError::io(&staging, source))?;

    let stage_result = (|| {
        atomic_copy(&bundle.dispatcher, &staging.join("codex-cli-editor.exe"))?;
        atomic_copy(&bundle.enhanced, &staging.join("codex.exe"))?;
        atomic_copy(&bundle.helper, &staging.join("codex-code-mode-host.exe"))?;
        atomic_copy(
            &bundle.vscode_extension,
            &staging.join("codex-cli-editor.vsix"),
        )?;
        atomic_copy(
            &bundle.manifest,
            &staging.join("compatibility-manifest.json"),
        )?;
        atomic_copy(
            &bundle.signature,
            &staging.join("compatibility-manifest.sig"),
        )?;
        verify_declared_artifact(
            &verified,
            "codex-cli-editor.exe",
            &staging.join("codex-cli-editor.exe"),
        )?;
        verify_declared_artifact(&verified, "codex-enhanced.exe", &staging.join("codex.exe"))?;
        verify_declared_artifact(
            &verified,
            "codex-code-mode-host.exe",
            &staging.join("codex-code-mode-host.exe"),
        )?;
        verify_declared_artifact(
            &verified,
            "codex-cli-editor.vsix",
            &staging.join("codex-cli-editor.vsix"),
        )?;
        Ok::<_, CodexCliEditorError>(())
    })();
    if let Err(error) = stage_result {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }

    let smoke_version = match smoke_probe(&staging.join("codex.exe")) {
        Ok(version) => version,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    let codex_version = normalized_version(&smoke_version).to_owned();
    if !verified.manifest.supports_codex(&codex_version) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(CodexCliEditorError::UnsupportedCodexVersion(codex_version));
    }

    crate::vscode::update_if_owned(&bundle.vscode_extension, prepared.vscode_extension_added)?;

    let rollback_directory = store.root().join(format!(
        ".update-rollback.{}.{:032x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let shim_staging = if dispatcher_changed {
        let shims = prepared
            .shim_directory
            .as_ref()
            .ok_or(CodexCliEditorError::NotInstalled)?;
        let directory = store.root().join(format!(
            ".shims-staging.{}.{:032x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir(&directory)
            .map_err(|source| CodexCliEditorError::io(&directory, source))?;
        atomic_copy(
            &staging.join("codex-cli-editor.exe"),
            &directory.join("codex-cli-editor.exe"),
        )?;
        if prepared.native_targets.contains_key(&CliKind::Codex) {
            atomic_copy(
                &staging.join("codex-cli-editor.exe"),
                &directory.join("codex.exe"),
            )?;
        }
        Some((shims.clone(), directory))
    } else {
        None
    };
    let shim_backup = store.root().join(format!(
        ".shims-rollback.{}.{:032x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let replacements = RefCell::new(Vec::new());
    let activated = RefCell::new(false);
    let shims_activated = Cell::new(false);
    let expected_install_id = prepared.install_id.clone();
    let expected_sequence = prepared.highest_manifest_sequence;
    let result = store.transaction(|current| {
        let mut state = current.ok_or(CodexCliEditorError::NotInstalled)?;
        if state.install_id != expected_install_id
            || state.highest_manifest_sequence != expected_sequence
        {
            return Err(CodexCliEditorError::StateChangedDuringOperation);
        }
        publish_staged_release(&staging, &final_directory)?;
        activated.replace(true);
        verify_release_bundle(
            &final_directory,
            &final_directory.join("codex-cli-editor.exe"),
            &final_directory.join("codex.exe"),
            &final_directory.join("codex-code-mode-host.exe"),
            &final_directory.join("compatibility-manifest.json"),
            &final_directory.join("compatibility-manifest.sig"),
            expected_sequence,
        )?;
        verify_declared_artifact(
            &verified,
            "codex-cli-editor.vsix",
            &final_directory.join("codex-cli-editor.vsix"),
        )?;
        std::fs::create_dir(&rollback_directory)
            .map_err(|source| CodexCliEditorError::io(&rollback_directory, source))?;

        let cache_directory = store.root().join("compatibility");
        let manifest_target = cache_directory.join("manifest.json");
        let signature_target = cache_directory.join("manifest.sig");
        backup_and_replace(
            &final_directory.join("compatibility-manifest.json"),
            &manifest_target,
            &rollback_directory,
            &replacements,
        )?;
        backup_and_replace(
            &final_directory.join("compatibility-manifest.sig"),
            &signature_target,
            &rollback_directory,
            &replacements,
        )?;

        if let Some((shims, replacement)) = &shim_staging {
            activate_shim_directory(shims, replacement, &shim_backup)?;
            shims_activated.set(true);
        }

        state.installed_version = VERSION.into();
        state.highest_manifest_sequence = verified.manifest.sequence;
        state.manifest_cache = Some(ManifestCacheRecord {
            manifest_path: manifest_target,
            signature_path: signature_target,
            sequence: verified.manifest.sequence,
            expires_unix: verified.manifest.expires_unix,
        });
        let enhanced_target = final_directory.join("codex.exe");
        let enhanced_artifact = verified
            .manifest
            .artifact("codex-enhanced.exe")
            .ok_or_else(|| CodexCliEditorError::ArtifactNotDeclared("codex-enhanced.exe".into()))?;
        let (_, modified_unix_ms) = artifact_metadata(&enhanced_target)?;
        state.active_release = Some(ReleaseRecord {
            version: release_name,
            directory: final_directory.clone(),
            codex_version,
            sha256: enhanced_artifact.sha256.clone(),
            file_size: enhanced_artifact.size,
            modified_unix_ms,
        });
        Ok((state, ()))
    });

    if result.is_err() {
        rollback_replacements(&replacements.into_inner());
        if shims_activated.get()
            && let Some((shims, _)) = &shim_staging
        {
            rollback_shim_directory(shims, &shim_backup);
        }
        if *activated.borrow() {
            let _ = std::fs::remove_dir_all(&final_directory);
        } else {
            let _ = std::fs::remove_dir_all(&staging);
        }
    }
    if let Some((_, replacement)) = &shim_staging {
        let _ = std::fs::remove_dir_all(replacement);
    }
    let _ = std::fs::remove_dir_all(&rollback_directory);
    result?;
    if let Err(error) = std::fs::remove_dir_all(&shim_backup)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!(
            "warning: the previous running shim remains at {} and can be removed after existing Codex sessions exit: {error}",
            shim_backup.display()
        );
    }
    prune_retained_releases(&versions_directory, &final_directory);
    println!(
        "Codex CLI Editor activated manifest sequence {} from {}",
        verified.manifest.sequence,
        bundle.directory().display()
    );
    Ok(())
}

fn publish_staged_release(staging: &Path, final_directory: &Path) -> Result<()> {
    publish_staged_release_with(staging, final_directory, |source, target| {
        std::fs::rename(source, target)
    })
}

fn publish_staged_release_with(
    staging: &Path,
    final_directory: &Path,
    rename: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> Result<()> {
    match rename(staging, final_directory) {
        Ok(()) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {}
        Err(source) => return Err(CodexCliEditorError::io(final_directory, source)),
    }

    std::fs::create_dir(final_directory)
        .map_err(|source| CodexCliEditorError::io(final_directory, source))?;
    let copy_result = (|| {
        for entry in
            std::fs::read_dir(staging).map_err(|source| CodexCliEditorError::io(staging, source))?
        {
            let entry = entry.map_err(|source| CodexCliEditorError::io(staging, source))?;
            let source = entry.path();
            let kind = entry
                .file_type()
                .map_err(|error| CodexCliEditorError::io(&source, error))?;
            if !kind.is_file() || kind.is_symlink() {
                return Err(CodexCliEditorError::UnsafeTarget(source));
            }
            atomic_copy(&source, &final_directory.join(entry.file_name()))?;
        }
        Ok(())
    })();
    if let Err(error) = copy_result {
        let _ = std::fs::remove_dir_all(final_directory);
        return Err(error);
    }
    if let Err(error) = std::fs::remove_dir_all(staging) {
        eprintln!(
            "warning: verified update staging residue remains at {}: {error}",
            staging.display()
        );
    }
    Ok(())
}

fn activate_shim_directory(shims: &Path, replacement: &Path, backup: &Path) -> Result<()> {
    std::fs::rename(shims, backup).map_err(|source| CodexCliEditorError::io(shims, source))?;
    if let Err(source) = std::fs::rename(replacement, shims) {
        let _ = std::fs::rename(backup, shims);
        return Err(CodexCliEditorError::io(shims, source));
    }
    Ok(())
}

fn rollback_shim_directory(shims: &Path, backup: &Path) {
    let failed = backup.with_extension("failed");
    if std::fs::rename(shims, &failed).is_ok() {
        if std::fs::rename(backup, shims).is_ok() {
            let _ = std::fs::remove_dir_all(failed);
        } else {
            let _ = std::fs::rename(failed, shims);
        }
    }
}

fn verify_managed_codex_compatibility(
    state: &State,
    manifest: &CompatibilityManifest,
) -> Result<()> {
    let Some(recorded_codex) = state.native_targets.get(&CliKind::Codex) else {
        return Ok(());
    };
    let version = normalized_version(&recorded_codex.version);
    if !manifest.supports_codex(version) {
        return Err(CodexCliEditorError::UnsupportedCodexVersion(version.into()));
    }
    Ok(())
}
fn quarantine_interrupted_release(versions: &Path, directory: &Path) -> Result<()> {
    let name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| CodexCliEditorError::UnsafeTarget(directory.to_path_buf()))?;
    let quarantine = versions.join(format!(
        ".interrupted.{name}.{}.{:032x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::rename(directory, &quarantine)
        .map_err(|source| CodexCliEditorError::io(directory, source))?;
    if let Err(error) = std::fs::remove_dir_all(&quarantine) {
        eprintln!(
            "warning: interrupted update residue remains quarantined at {}: {error}",
            quarantine.display()
        );
    }
    Ok(())
}

fn prune_retained_releases(versions: &Path, active_directory: &Path) {
    let Ok(entries) = std::fs::read_dir(versions) else {
        return;
    };
    let mut releases = Vec::new();
    for entry in entries.flatten() {
        let directory = entry.path();
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let Some(name) = directory.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with(".prune.") {
            if let Err(error) = std::fs::remove_dir_all(&directory) {
                eprintln!(
                    "warning: retained cleanup residue remains at {}: {error}",
                    directory.display()
                );
            }
            continue;
        }
        let manifest_path = directory.join("compatibility-manifest.json");
        let sequence = std::fs::read(&manifest_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|value| value.get("sequence")?.as_u64());
        if let Some(sequence) = sequence {
            releases.push((sequence, directory));
        }
    }
    releases.sort_by_key(|(sequence, _)| std::cmp::Reverse(*sequence));
    let mut prior_kept = 0usize;
    for (_, directory) in releases {
        if directory == active_directory {
            continue;
        }
        if prior_kept < 2 {
            prior_kept += 1;
            continue;
        }
        let name = directory.file_name().unwrap_or_default().to_string_lossy();
        let quarantine = versions.join(format!(".prune.{name}.{}", std::process::id()));
        if let Err(error) = std::fs::rename(&directory, &quarantine) {
            eprintln!(
                "warning: could not quarantine old release {}: {error}",
                directory.display()
            );
            continue;
        }
        if let Err(error) = std::fs::remove_dir_all(&quarantine) {
            eprintln!(
                "warning: old release cleanup is deferred at {}: {error}",
                quarantine.display()
            );
        }
    }
}

struct RollbackCandidate {
    directory: PathBuf,
    release_name: String,
    manifest_sequence: u64,
    expires_unix: u64,
    codex_version: String,
    codex_sha256: String,
    codex_file_size: u64,
    codex_modified_unix_ms: u128,
}

pub(crate) fn rollback(release: Option<&str>) -> Result<()> {
    let store = StateStore::for_current_user()?;
    rollback_with_store(&store, release, probe_release_version)
}

fn rollback_with_store<F>(store: &StateStore, release: Option<&str>, smoke_probe: F) -> Result<()>
where
    F: Fn(&Path) -> Result<String>,
{
    let prepared = store.load()?.ok_or(CodexCliEditorError::NotInstalled)?;
    let active = prepared
        .active_release
        .as_ref()
        .ok_or(CodexCliEditorError::EnhancedUnavailable)?;
    let active_sequence = prepared
        .manifest_cache
        .as_ref()
        .ok_or(CodexCliEditorError::EnhancedUnavailable)?
        .sequence;
    let versions = store.root().join("versions");
    let canonical_active = active
        .directory
        .canonicalize()
        .map_err(|source| CodexCliEditorError::io(&active.directory, source))?;
    let candidate = select_rollback_candidate(
        &versions,
        &canonical_active,
        active_sequence,
        prepared.highest_manifest_sequence,
        release,
        &smoke_probe,
    )?;

    let rollback_directory = store.root().join(format!(
        ".rollback-activation.{}.{:032x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let replacements = RefCell::new(Vec::new());
    let expected_install_id = prepared.install_id.clone();
    let expected_active = active.directory.clone();
    let expected_highest = prepared.highest_manifest_sequence;
    let result = store.transaction(|current| {
        let mut state = current.ok_or(CodexCliEditorError::NotInstalled)?;
        if state.install_id != expected_install_id
            || state.highest_manifest_sequence != expected_highest
            || state.active_release.as_ref().map(|item| &item.directory) != Some(&expected_active)
        {
            return Err(CodexCliEditorError::StateChangedDuringOperation);
        }
        let cache_directory = store.root().join("compatibility");
        let manifest_target = cache_directory.join("manifest.json");
        let signature_target = cache_directory.join("manifest.sig");
        backup_and_replace(
            &candidate.directory.join("compatibility-manifest.json"),
            &manifest_target,
            &rollback_directory,
            &replacements,
        )?;
        backup_and_replace(
            &candidate.directory.join("compatibility-manifest.sig"),
            &signature_target,
            &rollback_directory,
            &replacements,
        )?;
        state.manifest_cache = Some(ManifestCacheRecord {
            manifest_path: manifest_target,
            signature_path: signature_target,
            sequence: candidate.manifest_sequence,
            expires_unix: candidate.expires_unix,
        });
        state.active_release = Some(ReleaseRecord {
            version: candidate.release_name.clone(),
            directory: candidate.directory.clone(),
            codex_version: candidate.codex_version.clone(),
            sha256: candidate.codex_sha256.clone(),
            file_size: candidate.codex_file_size,
            modified_unix_ms: candidate.codex_modified_unix_ms,
        });
        Ok((state, ()))
    });
    if result.is_err() {
        rollback_replacements(&replacements.into_inner());
    }
    let _ = std::fs::remove_dir_all(&rollback_directory);
    result?;
    println!(
        "Codex CLI Editor rolled back enhanced Codex to {} (signed manifest sequence {}); update sequence protection remains at {}",
        candidate.release_name, candidate.manifest_sequence, expected_highest
    );
    Ok(())
}

fn select_rollback_candidate(
    versions: &Path,
    active_directory: &Path,
    active_sequence: u64,
    highest_sequence: u64,
    release: Option<&str>,
    smoke_probe: &impl Fn(&Path) -> Result<String>,
) -> Result<RollbackCandidate> {
    if let Some(release) = release {
        let component = Path::new(release);
        if component.components().count() != 1
            || component.file_name().and_then(|name| name.to_str()) != Some(release)
        {
            return Err(CodexCliEditorError::UnsafeTarget(versions.join(release)));
        }
        let directory = versions.join(release);
        let canonical_directory = directory
            .canonicalize()
            .map_err(|source| CodexCliEditorError::io(&directory, source))?;
        if canonical_directory == active_directory {
            return Err(CodexCliEditorError::NoRollbackAvailable);
        }
        return prepare_rollback_candidate(&canonical_directory, highest_sequence, smoke_probe);
    }

    let mut candidates = Vec::new();
    for entry in
        std::fs::read_dir(versions).map_err(|source| CodexCliEditorError::io(versions, source))?
    {
        let entry = entry.map_err(|source| CodexCliEditorError::io(versions, source))?;
        let directory = entry.path();
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let Ok(canonical_directory) = directory.canonicalize() else {
            continue;
        };
        if canonical_directory == active_directory {
            continue;
        }
        if let Ok(candidate) =
            prepare_rollback_candidate(&canonical_directory, highest_sequence, smoke_probe)
            && candidate.manifest_sequence < active_sequence
        {
            candidates.push(candidate);
        }
    }
    candidates
        .into_iter()
        .max_by_key(|candidate| candidate.manifest_sequence)
        .ok_or(CodexCliEditorError::NoRollbackAvailable)
}

fn prepare_rollback_candidate(
    directory: &Path,
    highest_sequence: u64,
    smoke_probe: &impl Fn(&Path) -> Result<String>,
) -> Result<RollbackCandidate> {
    let directory = directory
        .canonicalize()
        .map_err(|source| CodexCliEditorError::io(directory, source))?;
    let verified = verify_release_bundle(
        &directory,
        &directory.join("codex-cli-editor.exe"),
        &directory.join("codex.exe"),
        &directory.join("codex-code-mode-host.exe"),
        &directory.join("compatibility-manifest.json"),
        &directory.join("compatibility-manifest.sig"),
        0,
    )?;
    if verified.manifest.sequence > highest_sequence {
        return Err(CodexCliEditorError::ManifestRollback {
            highest: highest_sequence,
            received: verified.manifest.sequence,
        });
    }
    let probed = smoke_probe(&directory.join("codex.exe"))?;
    let codex_version = normalized_version(&probed).to_owned();
    if !verified.manifest.supports_codex(&codex_version) {
        return Err(CodexCliEditorError::UnsupportedCodexVersion(codex_version));
    }
    let release_name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| CodexCliEditorError::UnsafeTarget(directory.clone()))?
        .to_owned();
    let codex_path = directory.join("codex.exe");
    let codex_artifact = verified
        .manifest
        .artifact("codex-enhanced.exe")
        .ok_or_else(|| CodexCliEditorError::ArtifactNotDeclared("codex-enhanced.exe".into()))?;
    let (_, codex_modified_unix_ms) = artifact_metadata(&codex_path)?;
    Ok(RollbackCandidate {
        codex_sha256: codex_artifact.sha256.clone(),
        codex_file_size: codex_artifact.size,
        codex_modified_unix_ms,
        directory,
        release_name,
        manifest_sequence: verified.manifest.sequence,
        expires_unix: verified.manifest.expires_unix,
        codex_version,
    })
}

fn release_directory_name(sequence: u64) -> String {
    format!("{VERSION}-manifest-{sequence}")
}

fn backup_and_replace(
    source: &Path,
    target: &Path,
    rollback_directory: &Path,
    replacements: &RefCell<Vec<(PathBuf, Option<PathBuf>)>>,
) -> Result<()> {
    std::fs::create_dir_all(rollback_directory)
        .map_err(|source| CodexCliEditorError::io(rollback_directory, source))?;
    let index = replacements.borrow().len();
    let backup = if target.exists() {
        let backup = rollback_directory.join(index.to_string());
        std::fs::copy(target, &backup)
            .map_err(|source| CodexCliEditorError::io(&backup, source))?;
        Some(backup)
    } else {
        None
    };
    replacements
        .borrow_mut()
        .push((target.to_path_buf(), backup));
    atomic_copy(source, target)
}

fn rollback_replacements(replacements: &[(PathBuf, Option<PathBuf>)]) {
    for (target, backup) in replacements.iter().rev() {
        if let Some(backup) = backup {
            let _ = atomic_copy(backup, target);
        } else {
            let _ = std::fs::remove_file(target);
        }
    }
}
fn artifact_metadata(path: &Path) -> Result<(u64, u128)> {
    let metadata = path
        .metadata()
        .map_err(|source| CodexCliEditorError::io(path, source))?;
    let modified_unix_ms = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |value| value.as_millis());
    Ok((metadata.len(), modified_unix_ms))
}
fn verify_release_bundle(
    directory: &Path,
    dispatcher: &Path,
    enhanced: &Path,
    helper: &Path,
    manifest_path: &Path,
    signature_path: &Path,
    highest_sequence: u64,
) -> Result<VerifiedManifest> {
    if !cfg!(debug_assertions) && release_key_is_development() {
        return Err(CodexCliEditorError::DevelopmentKeyReleaseBlocked);
    }
    let bytes = std::fs::read(manifest_path)
        .map_err(|source| CodexCliEditorError::io(manifest_path, source))?;
    let signature = std::fs::read_to_string(signature_path)
        .map_err(|source| CodexCliEditorError::io(signature_path, source))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let verified = verify_manifest(&bytes, &signature, highest_sequence, now)?;
    if verified.freshness == Freshness::Expired {
        return Err(CodexCliEditorError::ManifestExpired);
    }
    if matches!(verified.freshness, Freshness::Grace { .. }) {
        eprintln!("warning: Codex CLI Editor compatibility manifest is stale but within grace");
    }
    let required = semver::Version::parse(&verified.manifest.minimum_dispatcher_version)
        .map_err(|_| CodexCliEditorError::InvalidManifestWindow)?;
    let current =
        semver::Version::parse(VERSION).map_err(|_| CodexCliEditorError::InvalidManifestWindow)?;
    if current < required {
        return Err(CodexCliEditorError::DispatcherTooOld {
            required: required.to_string(),
            current: current.to_string(),
        });
    }
    for (name, path) in [
        ("codex-cli-editor.exe", dispatcher),
        ("codex-enhanced.exe", enhanced),
        ("codex-code-mode-host.exe", helper),
    ] {
        if !path.is_file() {
            return Err(CodexCliEditorError::MissingReleaseArtifact(
                path.to_path_buf(),
            ));
        }
        verify_declared_artifact(&verified, name, path)?;
    }
    let _ = directory;
    Ok(verified)
}

fn verify_declared_artifact(verified: &VerifiedManifest, name: &str, path: &Path) -> Result<()> {
    let artifact = verified
        .manifest
        .artifact(name)
        .ok_or_else(|| CodexCliEditorError::ArtifactNotDeclared(name.into()))?;
    let metadata = path
        .metadata()
        .map_err(|source| CodexCliEditorError::io(path, source))?;
    if metadata.len() != artifact.size || sha256_file(path)? != artifact.sha256 {
        return Err(CodexCliEditorError::ArtifactVerificationFailed(
            path.to_path_buf(),
        ));
    }
    Ok(())
}

pub(crate) fn configure_default(target: crate::cli::DefaultTarget) -> Result<()> {
    let store = StateStore::for_current_user()?;
    store.transaction(|current| {
        let mut state = current.ok_or(CodexCliEditorError::NotInstalled)?;
        let _ = target;
        apply_default_selection(&mut state)?;
        Ok((state, ()))
    })
}

fn apply_default_selection(state: &mut State) -> Result<()> {
    if !state.native_targets.contains_key(&CliKind::Codex) {
        return Err(CodexCliEditorError::TargetNotFound(CliKind::Codex));
    }
    state.defaults.codex_enhanced = true;
    Ok(())
}

pub(crate) fn restore_defaults(target: crate::cli::DefaultTarget) -> Result<()> {
    let store = StateStore::for_current_user()?;
    store.transaction(|current| {
        let mut state = current.ok_or(CodexCliEditorError::NotInstalled)?;
        let _ = target;
        state.defaults.codex_enhanced = false;
        Ok((state, ()))
    })
}
pub(crate) fn repair(adopt_native: Option<crate::cli::DefaultTarget>) -> Result<()> {
    let _ = adopt_native.ok_or(CodexCliEditorError::RepairTargetRequired)?;
    let kind = CliKind::Codex;
    let store = StateStore::for_current_user()?;
    let prepared = store.load()?.ok_or(CodexCliEditorError::NotInstalled)?;
    let shim_directory = prepared
        .shim_directory
        .clone()
        .ok_or_else(|| CodexCliEditorError::UnsafeTarget(store.root().join("shims")))?;
    let expected = prepared.native_targets.get(&kind).cloned();
    let options = DiscoveryOptions::from_environment(shim_directory.clone())?;
    let discovered = discover_native(&options)?;
    let installed_dispatcher = shim_directory.join("codex-cli-editor.exe");
    let shim_name = "codex.exe";

    let was_missing = expected.is_none();
    let shim_target = shim_directory.join(shim_name);
    let result = store.transaction(|current| {
        let mut state = current.ok_or(CodexCliEditorError::NotInstalled)?;
        if state.native_targets.get(&kind).cloned() != expected {
            return Err(CodexCliEditorError::StateChangedDuringOperation);
        }
        let changed = adopt_discovered_target(&mut state, kind, discovered);
        if was_missing {
            atomic_copy(&installed_dispatcher, &shim_target)?;
        }
        Ok((state, changed))
    });
    if result.is_err() && was_missing {
        let _ = std::fs::remove_file(&shim_target);
    }
    let changed = result?;
    if was_missing {
        println!(
            "Codex CLI Editor added and adopted the native {} target",
            kind.as_str()
        );
    } else if changed {
        println!(
            "Codex CLI Editor adopted the revalidated native {} target",
            kind.as_str()
        );
    } else {
        println!(
            "Codex CLI Editor native {} target is already current",
            kind.as_str()
        );
    }
    Ok(())
}
pub(crate) fn adopt_in_place(expected: &crate::NativeTarget) -> Result<crate::NativeTarget> {
    let kind = CliKind::Codex;
    let discovered = refresh_recorded_target(expected)?;
    if discovered.path != expected.path
        || discovered.package_root != expected.package_root
        || !same_package_identity(&expected.package_identity, &discovered.package_identity)
    {
        return Err(CodexCliEditorError::TargetChanged(expected.path.clone()));
    }
    let store = StateStore::for_current_user()?;
    let adopted = discovered.clone();
    store.transaction(|current| {
        let mut state = current.ok_or(CodexCliEditorError::NotInstalled)?;
        let current_target = state
            .native_targets
            .get(&kind)
            .ok_or(CodexCliEditorError::TargetNotFound(kind))?;
        if current_target != expected {
            return Err(CodexCliEditorError::StateChangedDuringOperation);
        }
        if !adopt_discovered_target(&mut state, kind, discovered) {
            return Err(CodexCliEditorError::StateChangedDuringOperation);
        }
        Ok((state, ()))
    })?;
    eprintln!(
        "notice: Codex CLI Editor adopted an in-place {} update: {} -> {}",
        kind.as_str(),
        expected.version,
        adopted.version
    );
    Ok(adopted)
}
fn adopt_discovered_target(
    state: &mut State,
    kind: CliKind,
    discovered: crate::NativeTarget,
) -> bool {
    let Some(previous) = state.native_targets.get(&kind) else {
        state.native_targets.insert(kind, discovered);
        return true;
    };
    if previous == &discovered {
        return false;
    }
    let record = crate::AdoptionRecord {
        timestamp_unix_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        cli: kind,
        package_root: discovered.package_root.clone(),
        old_version: previous.version.clone(),
        new_version: discovered.version.clone(),
        old_sha256: previous.sha256.clone(),
        new_sha256: discovered.sha256.clone(),
    };
    state.native_targets.insert(kind, discovered);
    state.record_adoption(record);
    true
}

pub(crate) fn uninstall() -> Result<()> {
    let store = StateStore::for_current_user()?;
    let root = store.root().to_path_buf();
    store.remove_with(|state| {
        if let Err(error) = crate::vscode::uninstall_if_owned(state.vscode_extension_added) {
            eprintln!(
                "warning: {error}; core Codex CLI Editor cleanup will continue, but the VS Code extension may need manual removal"
            );
        }
        if state.path_entry_added
            && let (Some(snapshot), Some(shims)) =
                (&state.pre_install_user_path, &state.shim_directory)
        {
            restore_managed_user_path(snapshot, shims)?;
        }
        if !state.path_entry_added
            && let Some(shims) = &state.shim_directory
        {
            eprintln!(
                "notice: preserving pre-existing user PATH entry for {}; Codex CLI Editor did not add or own that setting",
                shims.display()
            );
        }
        if let Some(shims) = &state.shim_directory {
            remove_owned_shims(shims);
        }
        Ok(())
    })?;
    cleanup_owned_root(&root)?;
    Ok(())
}

fn remove_owned_shims(shims: &Path) {
    remove_owned_shims_with(shims, remove_or_defer);
}

fn remove_owned_shims_with(shims: &Path, mut remove: impl FnMut(&Path) -> Result<()>) {
    let mut deferred = false;
    for name in ["codex-cli-editor.exe", "codex.exe"] {
        if remove(&shims.join(name)).is_err() {
            deferred = true;
        }
    }
    if deferred {
        eprintln!(
            "warning: Codex CLI Editor state removal is continuing; any final shim residue will be reported after cleanup"
        );
    }
}

fn restore_managed_user_path(
    original: &crate::RegistryValueSnapshot,
    shim_directory: &Path,
) -> Result<()> {
    let current = read_user_path()?;
    let installed_value = prepend_shim(original, shim_directory)?;
    if current.existed
        && current.value_type == original.value_type
        && current.data == installed_value
    {
        return restore_user_path(original);
    }
    if !current.existed {
        return Ok(());
    }
    let without_shim = remove_shim(&current, shim_directory)?;
    if without_shim != current.data {
        write_user_path(current.value_type, &without_shim)?;
    }
    Ok(())
}

fn cleanup_owned_root(root: &Path) -> Result<()> {
    let expected = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .ok_or(CodexCliEditorError::StateDirectoryUnavailable)?
        .join("CodexCLIEditor");
    cleanup_owned_root_at(root, &expected)
}

fn cleanup_owned_root_at(root: &Path, expected: &Path) -> Result<()> {
    if root != expected || root.file_name().and_then(|name| name.to_str()) != Some("CodexCLIEditor")
    {
        return Err(CodexCliEditorError::UnsafeTarget(root.to_path_buf()));
    }
    if !root.exists() {
        return Ok(());
    }
    ensure_not_reparse(root)?;
    let mut directories = Vec::new();
    let mut pending = Vec::new();
    if let Err(error) = collect_owned_entries(root, &mut directories, &mut pending) {
        eprintln!(
            "warning: Codex CLI Editor state was removed but owned residue could not be enumerated: {error}"
        );
        return Ok(());
    }
    for path in pending {
        let _ = remove_or_defer(&path);
    }
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        if let Err(error) = std::fs::remove_dir(&directory)
            && error.kind() != std::io::ErrorKind::NotFound
            && error.kind() != std::io::ErrorKind::DirectoryNotEmpty
        {
            eprintln!(
                "warning: owned directory remains at {}: {error}",
                directory.display()
            );
        }
    }
    match std::fs::remove_dir(root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {
            report_owned_residue(root);
        }
        Err(error) => {
            eprintln!(
                "warning: owned directory remains at {}: {error}",
                root.display()
            );
        }
    }
    Ok(())
}

fn report_owned_residue(root: &Path) {
    let mut directories = Vec::new();
    let mut files = Vec::new();
    if collect_owned_entries(root, &mut directories, &mut files).is_err() {
        eprintln!(
            "warning: Codex CLI Editor state was removed but final residue could not be enumerated under {}",
            root.display()
        );
        return;
    }
    if files.is_empty() {
        eprintln!(
            "warning: Codex CLI Editor state was removed but an owned directory remains at {}",
            root.display()
        );
    }
    for path in files {
        eprintln!(
            "warning: Codex CLI Editor state was removed; delete final inert residue after this command exits: {}",
            path.display()
        );
    }
}

fn collect_owned_entries(
    directory: &Path,
    directories: &mut Vec<std::path::PathBuf>,
    files: &mut Vec<std::path::PathBuf>,
) -> Result<()> {
    for entry in
        std::fs::read_dir(directory).map_err(|source| CodexCliEditorError::io(directory, source))?
    {
        let entry = entry.map_err(|source| CodexCliEditorError::io(directory, source))?;
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|source| CodexCliEditorError::io(&path, source))?;
        if kind.is_dir() && !kind.is_symlink() {
            ensure_not_reparse(&path)?;
            collect_owned_entries(&path, directories, files)?;
            directories.push(path);
        } else {
            files.push(path);
        }
    }
    Ok(())
}
fn remove_or_defer(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) if matches!(source.raw_os_error(), Some(5 | 32)) => defer_delete(path),
        Err(source) => Err(CodexCliEditorError::io(path, source)),
    }
}

#[cfg(windows)]
fn defer_delete(path: &Path) -> Result<()> {
    if posix_delete(path).is_ok() {
        return Ok(());
    }

    use std::os::windows::ffi::OsStrExt;
    use std::ptr::null;
    use winapi::um::winbase::{
        MOVEFILE_DELAY_UNTIL_REBOOT, MOVEFILE_REPLACE_EXISTING, MoveFileExW,
    };

    let parent = path
        .parent()
        .ok_or_else(|| CodexCliEditorError::UnsafeTarget(path.to_path_buf()))?;
    let pending = parent.join(format!(
        ".pending-delete.{}.{}.exe",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let source_wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let pending_wide: Vec<u16> = pending.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: both paths are NUL terminated and remain alive for the call.
    if unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            pending_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING,
        )
    } == 0
    {
        let rename_error = std::io::Error::last_os_error();
        // SAFETY: source_wide is NUL terminated and a null destination requests deferred deletion.
        if unsafe { MoveFileExW(source_wide.as_ptr(), null(), MOVEFILE_DELAY_UNTIL_REBOOT) } == 0 {
            return Err(CodexCliEditorError::io(path, rename_error));
        }
        return Ok(());
    }

    // SAFETY: pending_wide is NUL terminated and a null destination requests deferred deletion.
    // The renamed path is the only possible residue. Cleanup reports its final name once.
    unsafe { MoveFileExW(pending_wide.as_ptr(), null(), MOVEFILE_DELAY_UNTIL_REBOOT) };
    Ok(())
}

#[cfg(windows)]
fn posix_delete(path: &Path) -> std::io::Result<()> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::null_mut;
    use winapi::shared::minwindef::DWORD;
    use winapi::um::fileapi::{CreateFileW, OPEN_EXISTING, SetFileInformationByHandle};
    use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
    use winapi::um::minwinbase::FileDispositionInfoEx;
    use winapi::um::winnt::{
        DELETE, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    #[repr(C)]
    struct FileDispositionInfoExData {
        flags: DWORD,
    }

    const FILE_DISPOSITION_DELETE: DWORD = 0x1;
    const FILE_DISPOSITION_POSIX_SEMANTICS: DWORD = 0x2;
    const FILE_DISPOSITION_IGNORE_READONLY_ATTRIBUTE: DWORD = 0x10;

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: wide is NUL terminated; the returned handle is closed below on every branch.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            DELETE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null_mut(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    let mut disposition = FileDispositionInfoExData {
        flags: FILE_DISPOSITION_DELETE
            | FILE_DISPOSITION_POSIX_SEMANTICS
            | FILE_DISPOSITION_IGNORE_READONLY_ATTRIBUTE,
    };
    // SAFETY: handle is valid and disposition points to the declared fixed-size buffer.
    let deleted = unsafe {
        SetFileInformationByHandle(
            handle,
            FileDispositionInfoEx,
            (&mut disposition as *mut FileDispositionInfoExData).cast::<c_void>(),
            std::mem::size_of::<FileDispositionInfoExData>() as DWORD,
        )
    };
    let error = if deleted == 0 {
        Some(std::io::Error::last_os_error())
    } else {
        None
    };
    // SAFETY: handle was returned by CreateFileW and has not been closed.
    unsafe { CloseHandle(handle) };
    error.map_or(Ok(()), Err)
}

#[cfg(not(windows))]
fn defer_delete(path: &Path) -> Result<()> {
    Err(CodexCliEditorError::io(
        path,
        std::io::Error::new(std::io::ErrorKind::PermissionDenied, "file is in use"),
    ))
}
fn atomic_copy(source: &Path, target: &Path) -> Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| CodexCliEditorError::UnsafeTarget(target.to_path_buf()))?;
    std::fs::create_dir_all(parent).map_err(|source| CodexCliEditorError::io(parent, source))?;
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        target.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    if temp.exists() {
        std::fs::remove_file(&temp).map_err(|source| CodexCliEditorError::io(&temp, source))?;
    }
    std::fs::copy(source, &temp).map_err(|source| CodexCliEditorError::io(&temp, source))?;
    replace_file(&temp, target).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::RegistryValueSnapshot;
    use crate::registry::prepend_shim;

    #[test]
    fn prepends_shim_without_losing_expandable_path_text() {
        let units: Vec<u16> = r"%USERPROFILE%\bin;C:\Tools"
            .encode_utf16()
            .chain([0])
            .collect();
        let snapshot = RegistryValueSnapshot {
            existed: true,
            value_type: 2,
            data: units.into_iter().flat_map(u16::to_le_bytes).collect(),
        };
        let bytes = prepend_shim(&snapshot, Path::new(r"C:\CodexCLIEditor\shims")).unwrap();
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|unit| *unit != 0)
            .collect();
        assert_eq!(
            String::from_utf16(&units).unwrap(),
            r"C:\CodexCLIEditor\shims;%USERPROFILE%\bin;C:\Tools"
        );
    }

    #[test]
    fn removes_only_the_owned_shim_from_a_changed_user_path() {
        let units: Vec<u16> = r"C:\Other;C:\CodexCLIEditor\shims;C:\Later"
            .encode_utf16()
            .chain([0])
            .collect();
        let snapshot = RegistryValueSnapshot {
            existed: true,
            value_type: 2,
            data: units.into_iter().flat_map(u16::to_le_bytes).collect(),
        };
        let bytes =
            crate::registry::remove_shim(&snapshot, Path::new(r"C:\CodexCLIEditor\shims")).unwrap();
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|unit| *unit != 0)
            .collect();
        assert_eq!(String::from_utf16(&units).unwrap(), r"C:\Other;C:\Later");
    }

    fn target(version: &str, hash: &str) -> crate::NativeTarget {
        crate::NativeTarget {
            path: std::path::PathBuf::from(r"C:\native\codex.exe"),
            package_root: std::path::PathBuf::from(r"C:\native"),
            package_identity: format!("codex:@openai/codex@{version}"),
            version: format!("codex-cli {version}"),
            sha256: hash.into(),
            file_size: 100,
            modified_unix_ms: 200,
        }
    }

    #[test]
    fn state_without_codex_can_accept_dispatcher_updates() {
        let mut state = crate::State::new("0.1.0");
        let manifest = crate::compatibility::CompatibilityManifest {
            schema_version: 1,
            sequence: 2,
            issued_unix: 100,
            expires_unix: 200,
            minimum_dispatcher_version: "0.1.0".into(),
            compatibility: Vec::new(),
            artifacts: Vec::new(),
        };

        super::verify_managed_codex_compatibility(&state, &manifest).unwrap();
        state
            .native_targets
            .insert(crate::CliKind::Codex, target("0.148.0", "codex"));
        assert!(matches!(
            super::verify_managed_codex_compatibility(&state, &manifest),
            Err(crate::CodexCliEditorError::UnsupportedCodexVersion(version))
                if version == "0.148.0"
        ));
    }
    #[test]
    fn install_notice_distinguishes_a_newer_extracted_bundle() {
        assert!(super::newer_bundle_available(12, 11));
        assert!(!super::newer_bundle_available(11, 11));
        assert!(!super::newer_bundle_available(10, 11));
    }

    #[test]
    fn install_recognizes_the_installed_codex_cli_editor_shim() {
        let directory = crate::test_support::TempDir::new();
        let shims = directory.path().join("shims");
        std::fs::create_dir(&shims).unwrap();
        let editor = shims.join("codex-cli-editor.exe");
        std::fs::write(&editor, b"shim").unwrap();
        let shims = shims.canonicalize().unwrap();
        let editor = editor.canonicalize().unwrap();
        let mut state = crate::State::new("0.1.0");
        state.shim_directory = Some(shims);

        assert!(super::is_installed_codex_cli_editor_shim(&state, &editor));
        assert!(!super::is_installed_codex_cli_editor_shim(
            &state,
            &directory.path().join("external-codex-cli-editor.exe")
        ));
    }

    #[test]
    fn shim_removal_failures_do_not_abort_state_cleanup() {
        let directory = crate::test_support::TempDir::new();
        let shims = directory.path().join("shims");
        let mut attempted = Vec::new();

        super::remove_owned_shims_with(&shims, |path| {
            attempted.push(path.to_path_buf());
            Err(crate::CodexCliEditorError::io(
                path,
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, "in use"),
            ))
        });

        assert_eq!(
            attempted,
            ["codex-cli-editor.exe", "codex.exe"].map(|name| shims.join(name))
        );
    }

    #[cfg(windows)]
    #[test]
    fn posix_delete_removes_a_file_from_the_visible_namespace() {
        let directory = crate::test_support::TempDir::new();
        let file = directory.path().join("owned.exe");
        std::fs::write(&file, b"owned").unwrap();

        super::posix_delete(&file).unwrap();

        assert!(!file.exists());
    }

    #[cfg(windows)]
    #[test]
    fn in_use_delete_removes_the_running_executable_from_its_command_path() {
        const CHILD_MARKER_ENV: &str = "CODEX_CLI_EDITOR_POSIX_DELETE_CHILD_MARKER";
        if let Some(marker) = std::env::var_os(CHILD_MARKER_ENV) {
            std::fs::write(marker, b"ready").unwrap();
            std::thread::sleep(std::time::Duration::from_secs(2));
            return;
        }

        let directory = crate::test_support::TempDir::new();
        let running = directory.path().join("running-test.exe");
        let marker = directory.path().join("child-ready");
        std::fs::copy(std::env::current_exe().unwrap(), &running).unwrap();
        let mut child = std::process::Command::new(&running)
            .args([
                "--exact",
                "installer::tests::in_use_delete_removes_the_running_executable_from_its_command_path",
            ])
            .env(CHILD_MARKER_ENV, &marker)
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(marker.exists(), "child executable did not become ready");

        super::defer_delete(&running).unwrap();

        assert!(!running.exists());
        let pending = std::fs::read_dir(directory.path())
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(".pending-delete."))
            });
        assert!(pending.is_some());
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn cleanup_only_removes_the_exact_owned_root() {
        let directory = crate::test_support::TempDir::new();
        let root = directory.path().join("CodexCLIEditor");
        let versions = root.join("versions");
        let logs = root.join("logs");
        std::fs::create_dir_all(&versions).unwrap();
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(versions.join("artifact.exe"), b"owned").unwrap();
        std::fs::write(logs.join("audit.log"), b"owned").unwrap();

        super::cleanup_owned_root_at(&root, &root).unwrap();
        assert!(!root.exists());
        let wrong = directory.path().join("NotCodexCLIEditor");
        assert!(matches!(
            super::cleanup_owned_root_at(&wrong, &root),
            Err(crate::CodexCliEditorError::UnsafeTarget(path)) if path == wrong
        ));
    }

    #[test]
    fn interrupted_release_directory_is_quarantined_before_retry() {
        let directory = crate::test_support::TempDir::new();
        let versions = directory.path().join("versions");
        let interrupted = versions.join("0.1.0-manifest-8");
        std::fs::create_dir_all(&interrupted).unwrap();
        std::fs::write(interrupted.join("partial.exe"), b"partial").unwrap();

        super::quarantine_interrupted_release(&versions, &interrupted).unwrap();

        assert!(!interrupted.exists());
        assert_eq!(std::fs::read_dir(&versions).unwrap().count(), 0);
    }

    #[test]
    fn release_publication_copies_verified_files_when_directory_rename_is_denied() {
        let directory = crate::test_support::TempDir::new();
        let staging = directory.path().join("staging");
        let published = directory.path().join("published");
        std::fs::create_dir(&staging).unwrap();
        std::fs::write(staging.join("artifact.exe"), b"verified").unwrap();

        super::publish_staged_release_with(&staging, &published, |_, _| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "directory rename denied",
            ))
        })
        .unwrap();

        assert_eq!(
            std::fs::read(published.join("artifact.exe")).unwrap(),
            b"verified"
        );
        assert!(!staging.exists());
    }

    #[test]
    fn shim_directory_swap_preserves_the_previous_dispatcher_for_cleanup() {
        let directory = crate::test_support::TempDir::new();
        let shims = directory.path().join("shims");
        let replacement = directory.path().join("replacement");
        let backup = directory.path().join("backup");
        std::fs::create_dir(&shims).unwrap();
        std::fs::create_dir(&replacement).unwrap();
        std::fs::write(shims.join("codex.exe"), b"old").unwrap();
        std::fs::write(replacement.join("codex.exe"), b"new").unwrap();

        super::activate_shim_directory(&shims, &replacement, &backup).unwrap();

        assert_eq!(std::fs::read(shims.join("codex.exe")).unwrap(), b"new");
        assert_eq!(std::fs::read(backup.join("codex.exe")).unwrap(), b"old");
        assert!(!replacement.exists());
    }

    #[cfg(windows)]
    #[test]
    fn shim_directory_swap_succeeds_while_the_previous_dispatcher_is_running() {
        const CHILD_MARKER_ENV: &str = "CODEX_CLI_EDITOR_SHIM_SWAP_CHILD_MARKER";
        if let Some(marker) = std::env::var_os(CHILD_MARKER_ENV) {
            std::fs::write(marker, b"ready").unwrap();
            std::thread::sleep(std::time::Duration::from_secs(2));
            return;
        }

        let directory = crate::test_support::TempDir::new();
        let shims = directory.path().join("shims");
        let replacement = directory.path().join("replacement");
        let backup = directory.path().join("backup");
        let marker = directory.path().join("child-ready");
        std::fs::create_dir(&shims).unwrap();
        std::fs::create_dir(&replacement).unwrap();
        let running = shims.join("codex.exe");
        std::fs::copy(std::env::current_exe().unwrap(), &running).unwrap();
        std::fs::write(replacement.join("codex.exe"), b"new").unwrap();
        let mut child = std::process::Command::new(&running)
            .args([
                "--exact",
                "installer::tests::shim_directory_swap_succeeds_while_the_previous_dispatcher_is_running",
            ])
            .env(CHILD_MARKER_ENV, &marker)
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(marker.exists(), "child dispatcher did not become ready");

        super::activate_shim_directory(&shims, &replacement, &backup).unwrap();

        assert_eq!(std::fs::read(shims.join("codex.exe")).unwrap(), b"new");
        assert!(backup.join("codex.exe").exists());
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn default_selection_requires_native_codex() {
        let mut state = crate::State::new("0.1.0");
        assert!(matches!(
            super::apply_default_selection(&mut state),
            Err(crate::CodexCliEditorError::TargetNotFound(
                crate::CliKind::Codex
            ))
        ));

        state
            .native_targets
            .insert(crate::CliKind::Codex, target("0.148.0", "codex"));
        super::apply_default_selection(&mut state).unwrap();
        assert!(state.defaults.codex_enhanced);
    }

    #[test]
    fn adopting_native_target_records_a_bounded_audit_entry() {
        let mut state = crate::State::new("0.1.0");
        state
            .native_targets
            .insert(crate::CliKind::Codex, target("0.148.0", "old"));
        let replacement = target("0.149.0", "new");

        assert!(super::adopt_discovered_target(
            &mut state,
            crate::CliKind::Codex,
            replacement.clone()
        ));
        assert_eq!(
            state.native_targets.get(&crate::CliKind::Codex),
            Some(&replacement)
        );
        assert_eq!(state.adoption_history.len(), 1);
        let record = &state.adoption_history[0];
        assert_eq!(record.old_version, "codex-cli 0.148.0");
        assert_eq!(record.new_version, "codex-cli 0.149.0");
        assert_eq!(record.old_sha256, "old");
        assert_eq!(record.new_sha256, "new");
        assert!(!super::adopt_discovered_target(
            &mut state,
            crate::CliKind::Codex,
            replacement
        ));
        assert_eq!(state.adoption_history.len(), 1);
    }

    #[test]
    fn release_bundle_verification_detects_artifact_tampering() {
        use ed25519_dalek::{Signer, SigningKey};

        use crate::compatibility::{Artifact, CompatibilityManifest};
        use crate::discovery::sha256_file;

        let directory = crate::test_support::TempDir::new();
        let dispatcher = directory.path().join("codex-cli-editor.exe");
        let enhanced = directory.path().join("codex-enhanced.exe");
        let helper = directory.path().join("codex-code-mode-host.exe");
        let vscode_extension = directory.path().join("codex-cli-editor.vsix");
        std::fs::write(&dispatcher, b"dispatcher").unwrap();
        std::fs::write(&enhanced, b"enhanced").unwrap();
        std::fs::write(&helper, b"helper").unwrap();
        std::fs::write(&vscode_extension, b"vsix").unwrap();
        let artifacts = [
            ("codex-cli-editor.exe", &dispatcher),
            ("codex-enhanced.exe", &enhanced),
            ("codex-code-mode-host.exe", &helper),
            ("codex-cli-editor.vsix", &vscode_extension),
        ]
        .into_iter()
        .map(|(name, path)| Artifact {
            name: name.into(),
            url: format!("https://example.invalid/{name}"),
            sha256: sha256_file(path).unwrap(),
            size: path.metadata().unwrap().len(),
        })
        .collect();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let manifest = CompatibilityManifest {
            schema_version: 1,
            sequence: 7,
            issued_unix: now,
            expires_unix: now + 60,
            minimum_dispatcher_version: env!("CARGO_PKG_VERSION").into(),
            compatibility: Vec::new(),
            artifacts,
        };
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let manifest_path = directory.path().join("compatibility-manifest.json");
        let signature_path = directory.path().join("compatibility-manifest.sig");
        std::fs::write(&manifest_path, &bytes).unwrap();
        let seed: [u8; 32] =
            hex::decode("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
                .unwrap()
                .try_into()
                .unwrap();
        let signature = SigningKey::from_bytes(&seed).sign(&bytes);
        std::fs::write(&signature_path, hex::encode(signature.to_bytes())).unwrap();

        super::verify_release_bundle(
            directory.path(),
            &dispatcher,
            &enhanced,
            &helper,
            &manifest_path,
            &signature_path,
            6,
        )
        .unwrap();

        std::fs::write(&enhanced, b"tampered").unwrap();
        assert!(matches!(
            super::verify_release_bundle(
                directory.path(),
                &dispatcher,
                &enhanced,
                &helper,
                &manifest_path,
                &signature_path,
                6,
            ),
            Err(crate::CodexCliEditorError::ArtifactVerificationFailed(path)) if path == enhanced
        ));
    }

    #[test]
    fn signed_update_and_rollback_preserve_sequence_and_previous_release() {
        use ed25519_dalek::{Signer, SigningKey};

        use crate::compatibility::{Artifact, CompatibilityEntry, CompatibilityManifest};
        use crate::discovery::sha256_file;
        use crate::state::{ManifestCacheRecord, ReleaseRecord, StateStore};

        let directory = crate::test_support::TempDir::new();
        let store = StateStore::new(directory.path().join("store"));
        let old_release = store.root().join("versions").join("old");
        let shims = store.root().join("shims");
        std::fs::create_dir_all(&old_release).unwrap();
        std::fs::create_dir_all(&shims).unwrap();
        let current = std::env::current_exe().unwrap().canonicalize().unwrap();
        std::fs::copy(&current, old_release.join("codex-cli-editor.exe")).unwrap();
        std::fs::write(old_release.join("codex.exe"), b"old enhanced").unwrap();
        for name in ["codex-cli-editor.exe", "codex.exe"] {
            std::fs::copy(&current, shims.join(name)).unwrap();
        }
        let compatibility = store.root().join("compatibility");
        std::fs::create_dir_all(&compatibility).unwrap();
        let old_manifest = compatibility.join("manifest.json");
        let old_signature = compatibility.join("manifest.sig");
        std::fs::write(&old_manifest, b"old manifest").unwrap();
        std::fs::write(&old_signature, b"old signature").unwrap();

        let mut state = crate::State::new("0.1.0");
        state.shim_directory = Some(shims);
        state.native_targets.insert(
            crate::CliKind::Codex,
            super::tests::target("probe", "native"),
        );
        state.active_release = Some(ReleaseRecord {
            version: "old".into(),
            directory: old_release.clone(),
            codex_version: "probe".into(),
            sha256: sha256_file(&old_release.join("codex.exe")).unwrap(),
            file_size: 0,
            modified_unix_ms: 0,
        });
        state.highest_manifest_sequence = 1;
        state.manifest_cache = Some(ManifestCacheRecord {
            manifest_path: old_manifest,
            signature_path: old_signature,
            sequence: 1,
            expires_unix: u64::MAX,
        });

        let bundle = directory.path().join("bundle");
        std::fs::create_dir(&bundle).unwrap();
        let dispatcher = bundle.join("codex-cli-editor.exe");
        let enhanced = bundle.join("codex-enhanced.exe");
        let helper = bundle.join("codex-code-mode-host.exe");
        let vscode_extension = bundle.join("codex-cli-editor.vsix");
        std::fs::copy(&current, &dispatcher).unwrap();
        std::fs::copy(&current, &enhanced).unwrap();
        std::fs::write(&helper, b"helper").unwrap();
        std::fs::write(&vscode_extension, b"vsix").unwrap();
        let codex_version = "0.149.0".to_owned();
        state
            .native_targets
            .get_mut(&crate::CliKind::Codex)
            .unwrap()
            .version = format!("codex-cli {codex_version}");
        store.save(&state).unwrap();

        let artifacts = [
            ("codex-cli-editor.exe", &dispatcher),
            ("codex-enhanced.exe", &enhanced),
            ("codex-code-mode-host.exe", &helper),
            ("codex-cli-editor.vsix", &vscode_extension),
        ]
        .into_iter()
        .map(|(name, path)| Artifact {
            name: name.into(),
            url: format!("https://example.invalid/{name}"),
            sha256: sha256_file(path).unwrap(),
            size: path.metadata().unwrap().len(),
        })
        .collect();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let manifest = CompatibilityManifest {
            schema_version: 1,
            sequence: 2,
            issued_unix: now,
            expires_unix: now + 60,
            minimum_dispatcher_version: env!("CARGO_PKG_VERSION").into(),
            compatibility: vec![CompatibilityEntry {
                codex: codex_version.clone(),
                vscode: vec!["test".into()],
            }],
            artifacts,
        };
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let manifest_path = bundle.join("compatibility-manifest.json");
        std::fs::write(&manifest_path, &bytes).unwrap();
        let seed: [u8; 32] =
            hex::decode("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
                .unwrap()
                .try_into()
                .unwrap();
        let signature = SigningKey::from_bytes(&seed).sign(&bytes);
        std::fs::write(
            bundle.join("compatibility-manifest.sig"),
            hex::encode(signature.to_bytes()),
        )
        .unwrap();

        super::update_with_store(
            &bundle,
            &store,
            &current,
            |_| Ok("codex-cli 0.149.0".into()),
        )
        .unwrap();
        let updated = store.load().unwrap().unwrap();
        assert_eq!(updated.highest_manifest_sequence, 2);
        assert_eq!(
            updated.active_release.as_ref().unwrap().codex_version,
            codex_version
        );
        let active_release = updated.active_release.as_ref().unwrap();
        let declared_enhanced = manifest.artifact("codex-enhanced.exe").unwrap();
        assert!(active_release.directory.is_dir());
        assert_eq!(active_release.sha256, declared_enhanced.sha256);
        assert_eq!(active_release.file_size, declared_enhanced.size);
        assert!(old_release.is_dir());
        assert_eq!(
            std::fs::read(
                updated
                    .manifest_cache
                    .as_ref()
                    .unwrap()
                    .manifest_path
                    .clone()
            )
            .unwrap(),
            bytes
        );

        let rollback_release = store.root().join("versions").join("rollback-release");
        std::fs::create_dir(&rollback_release).unwrap();
        for name in [
            "codex-cli-editor.exe",
            "codex.exe",
            "codex-code-mode-host.exe",
        ] {
            std::fs::copy(&current, rollback_release.join(name)).unwrap();
        }
        let rollback_artifacts = [
            (
                "codex-cli-editor.exe",
                rollback_release.join("codex-cli-editor.exe"),
            ),
            ("codex-enhanced.exe", rollback_release.join("codex.exe")),
            (
                "codex-code-mode-host.exe",
                rollback_release.join("codex-code-mode-host.exe"),
            ),
        ]
        .into_iter()
        .map(|(name, path)| Artifact {
            name: name.into(),
            url: format!("https://example.invalid/{name}"),
            sha256: sha256_file(&path).unwrap(),
            size: path.metadata().unwrap().len(),
        })
        .collect();
        let rollback_manifest = CompatibilityManifest {
            schema_version: 1,
            sequence: 1,
            issued_unix: now,
            expires_unix: now + 60,
            minimum_dispatcher_version: env!("CARGO_PKG_VERSION").into(),
            compatibility: vec![CompatibilityEntry {
                codex: "0.148.0".into(),
                vscode: vec!["test".into()],
            }],
            artifacts: rollback_artifacts,
        };
        let rollback_bytes = serde_json::to_vec(&rollback_manifest).unwrap();
        std::fs::write(
            rollback_release.join("compatibility-manifest.json"),
            &rollback_bytes,
        )
        .unwrap();
        let rollback_signature = SigningKey::from_bytes(&seed).sign(&rollback_bytes);
        std::fs::write(
            rollback_release.join("compatibility-manifest.sig"),
            hex::encode(rollback_signature.to_bytes()),
        )
        .unwrap();

        super::rollback_with_store(&store, None, |_| Ok("codex-cli 0.148.0".into())).unwrap();
        let rolled_back = store.load().unwrap().unwrap();
        assert_eq!(rolled_back.highest_manifest_sequence, 2);
        assert_eq!(rolled_back.manifest_cache.as_ref().unwrap().sequence, 1);
        let rolled_back_release = rolled_back.active_release.as_ref().unwrap();
        let declared_rollback = rollback_manifest.artifact("codex-enhanced.exe").unwrap();
        assert_eq!(
            rolled_back_release.directory,
            rollback_release.canonicalize().unwrap()
        );
        assert_eq!(rolled_back_release.sha256, declared_rollback.sha256);
        assert_eq!(rolled_back_release.file_size, declared_rollback.size);
    }
}
