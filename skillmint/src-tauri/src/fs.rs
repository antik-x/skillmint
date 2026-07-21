use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

/// Create symlink on Unix; fallback to copy on error.
///
/// The silent copy fallback is intended for project-level diff resolution
/// (`resolve_skill_diff`) and install/binding paths. The center<->agent sync
/// engine uses [`create_symlink_strict`] instead, so a symlink failure is
/// reported rather than quietly materializing a full copy (P0-1).
pub fn create_symlink_or_copy(src: &Path, dst: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        if dst.exists() || dst.is_symlink() {
            remove_path(dst)?;
        }
        if let Err(_) = std::os::unix::fs::symlink(src, dst) {
            copy_dir_all(src, dst)?;
        }
    }
    #[cfg(not(unix))]
    {
        copy_dir_all(src, dst)?;
    }
    Ok(())
}

/// Create a symlink with no silent copy fallback (P0-1).
///
/// On Unix a symlink failure is returned to the caller so it can surface in
/// `SyncReport.broken`; copy only ever happens for `mode = copy` targets.
/// Non-Unix builds keep the copy fallback (no symlink support).
pub fn create_symlink_strict(src: &Path, dst: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        if dst.exists() || dst.is_symlink() {
            remove_path(dst)?;
        }
        std::os::unix::fs::symlink(src, dst)?;
    }
    #[cfg(not(unix))]
    {
        copy_dir_all(src, dst)?;
    }
    Ok(())
}

/// Remove a path, whether it's a file, directory or symlink.
pub fn remove_path(path: &Path) -> Result<()> {
    if path.is_dir() && !path.is_symlink() {
        fs::remove_dir_all(path)?;
    } else if path.exists() || path.is_symlink() {
        fs::remove_file(path)?;
    }
    Ok(())
}

/// Atomically replace `dst` with `src` by renaming `dst` out of the way first.
/// Works for files, directories and symlinks. On failure a best-effort restore
/// of the previous `dst` is attempted. After success, `src` no longer exists at
/// its original path.
pub fn replace_path_atomic(src: &Path, dst: &Path) -> Result<()> {
    if !src.exists() && !src.is_symlink() {
        anyhow::bail!("replace_path_atomic source does not exist: {}", src.display());
    }
    let backup = dst.with_extension("skillmint-atomic-backup");
    if backup.exists() || backup.is_symlink() {
        remove_path(&backup)?;
    }
    let had_dst = dst.exists() || dst.is_symlink();
    if had_dst {
        fs::rename(dst, &backup)?;
    }
    if let Err(e) = fs::rename(src, dst) {
        if had_dst && (backup.exists() || backup.is_symlink()) && !dst.exists() && !dst.is_symlink() {
            let _ = fs::rename(&backup, dst);
        }
        return Err(e.into());
    }
    if backup.exists() || backup.is_symlink() {
        let _ = remove_path(&backup);
    }
    Ok(())
}

/// Copy a directory recursively.
pub fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> io::Result<()> {
    fs::create_dir_all(&dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// Compute a stable hash for a file or directory.
pub fn compute_hash(path: &Path) -> Result<String> {
    if path.is_symlink() {
        // Hash symlink target
        let target = fs::read_link(path)?;
        return Ok(format!(
            "{:x}",
            Sha256::digest(target.to_string_lossy().as_bytes())
        ));
    }

    if path.is_file() {
        let content = fs::read(path)?;
        return Ok(format!("{:x}", Sha256::digest(&content)));
    }

    if path.is_dir() {
        let mut hashes: Vec<String> = Vec::new();
        for entry in WalkDir::new(path).sort_by_file_name() {
            let entry = entry?;
            if entry.file_type().is_file() {
                let rel = entry.path().strip_prefix(path).unwrap_or(entry.path());
                let content = fs::read(entry.path())?;
                hashes.push(format!("{}:{:x}", rel.display(), Sha256::digest(&content)));
            }
        }
        let joined = hashes.join("\n");
        return Ok(format!("{:x}", Sha256::digest(joined.as_bytes())));
    }

    Ok(String::new())
}

/// Check whether a path is a broken symlink.
pub fn is_broken_symlink(path: &Path) -> bool {
    path.is_symlink() && !path.exists()
}

/// P0-2: true when `path` is a symlink whose canonical target equals the
/// canonical form of `target` (e.g. an agent-side link into the center repo).
/// Broken links and plain directories return false.
pub fn is_symlink_to(path: &Path, target: &Path) -> bool {
    if !path.is_symlink() {
        return false;
    }
    match (std::fs::canonicalize(path), std::fs::canonicalize(target)) {
        (Ok(resolved), Ok(want)) => resolved == want,
        _ => false,
    }
}

/// P1-4: rewrite the `name:` field of a SKILL.md YAML front matter from `old`
/// to `new`. Only fires when the file has a front matter block AND the name
/// exactly matches `old` (repo convention: directory name == front matter
/// name). Returns true when the file was rewritten; every other line is
/// preserved verbatim.
pub fn rewrite_skill_md_name(skill_md: &Path, old: &str, new: &str) -> Result<bool> {
    if !skill_md.is_file() {
        return Ok(false);
    }
    let content = std::fs::read_to_string(skill_md)?;
    let mut lines = content.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Ok(false);
    }
    let mut out: Vec<String> = vec!["---".to_string()];
    let mut in_front_matter = true;
    let mut changed = false;
    for line in lines {
        let trimmed = line.trim_end();
        if in_front_matter {
            if trimmed == "---" {
                in_front_matter = false;
            } else if !changed {
                if let Some(value) = trimmed.strip_prefix("name:") {
                    let value = value.trim().trim_matches('"').trim_matches('\'');
                    if value == old {
                        out.push(format!("name: {}", new));
                        changed = true;
                        continue;
                    }
                }
            }
        }
        out.push(line.to_string());
    }
    if changed {
        let mut body = out.join("\n");
        if content.ends_with('\n') {
            body.push('\n');
        }
        std::fs::write(skill_md, body)?;
    }
    Ok(changed)
}

/// Resolve symlink target, returning original path if not a symlink.
pub fn resolve_symlink(path: &Path) -> PathBuf {
    if path.is_symlink() {
        fs::read_link(path).unwrap_or_else(|_| path.to_path_buf())
    } else {
        path.to_path_buf()
    }
}

/// Move a directory into center repo, backing up conflicts with `.backup` suffix.
pub fn move_into_center(source: &Path, center_path: &Path) -> Result<PathBuf> {
    let name = source.file_name().context("source has no file name")?;
    let dest = center_path.join(name);
    if dest.exists() {
        let backup = center_path.join(format!("{}.backup", name.to_string_lossy()));
        if backup.exists() {
            fs::remove_dir_all(&backup)?;
        }
        fs::rename(&dest, &backup)?;
    }
    fs::rename(source, &dest)?;
    Ok(dest)
}

// =============================================================================
// PRD-01 patch FR-C/FR-F: multi-version skill helpers
// =============================================================================

/// The version label representing the mutable, always-current version.
pub const LATEST_VERSION: &str = "latest";

/// Resolve the effective skill directory for a given version.
///
/// Multi-version layout (FR-F):
///   `<skill_root>/latest/`      ← mutable, current
///   `<skill_root>/v1/`, `v2/`…  ← immutable snapshots
///
/// Legacy flat layout (no `latest/` dir) is treated as a degenerate `latest`:
/// the skill root itself is returned. This keeps existing skills working with
/// zero migration.
pub fn get_effective_skill_dir(skill_root: &Path, version: Option<&str>) -> PathBuf {
    let latest_dir = skill_root.join(LATEST_VERSION);
    let has_versions = latest_dir.is_dir();
    let label = version.unwrap_or(LATEST_VERSION);
    if has_versions {
        skill_root.join(label)
    } else {
        // Flat layout: only `latest` is meaningful; ignore pinned labels.
        skill_root.to_path_buf()
    }
}

/// Does this skill root use the multi-version layout?
pub fn is_multi_version(skill_root: &Path) -> bool {
    skill_root.join(LATEST_VERSION).is_dir()
}

/// Sidecar filename holding a version note (PRD §4.5c, DECISIONS 14).
pub const VERSION_NOTE_FILE: &str = ".version-note";

/// List all versions of a skill (latest first, then v<x> descending).
/// Each entry: (version label, mtime, optional note read from sidecar).
pub fn list_skill_versions(skill_root: &Path) -> Result<Vec<(String, u64, Option<String>)>> {
    let mut out = Vec::new();
    if !skill_root.is_dir() {
        return Ok(out);
    }
    // If multi-version layout, scan subdirs named latest / v<int>.
    let latest_dir = skill_root.join(LATEST_VERSION);
    if latest_dir.is_dir() {
        if let Ok(meta) = fs::metadata(&latest_dir) {
            // latest never carries a note (it's a mutable pointer).
            out.push((LATEST_VERSION.to_string(), mtime_secs(&meta), None));
        }
        for entry in fs::read_dir(skill_root)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('v') {
                if let Some(n) = name[1..].parse::<u32>().ok() {
                    let mtime = entry.metadata().map(|m| mtime_secs(&m)).unwrap_or(0);
                    let note = read_version_note(&entry.path());
                    out.push((format!("v{}", n), mtime, note));
                }
            }
        }
        // Sort: latest first, then vN descending (latest = i64::MAX sorts first).
        out.sort_by(|a, b| version_sort_key(&b.0).cmp(&version_sort_key(&a.0)));
    } else {
        // Flat layout: a single implicit "latest".
        if let Ok(meta) = fs::metadata(skill_root) {
            out.push((LATEST_VERSION.to_string(), mtime_secs(&meta), None));
        }
    }
    Ok(out)
}

/// Read the note sidecar of a version directory (None if absent).
fn read_version_note(version_dir: &Path) -> Option<String> {
    let note_path = version_dir.join(VERSION_NOTE_FILE);
    fs::read_to_string(&note_path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Write the note sidecar into a version directory (no-op for latest; best-effort).
pub fn write_version_note(version_dir: &Path, note: Option<&str>) -> Result<()> {
    let note_path = version_dir.join(VERSION_NOTE_FILE);
    match note {
        Some(n) if !n.trim().is_empty() => {
            fs::write(&note_path, n)?;
        }
        _ => {
            // Clear: remove sidecar if it exists.
            if note_path.exists() {
                fs::remove_file(&note_path)?;
            }
        }
    }
    Ok(())
}

/// Numeric sort key: latest = i64::MAX, vN = N (so latest sorts first when desc).
fn version_sort_key(label: &str) -> i64 {
    if label == LATEST_VERSION {
        i64::MAX
    } else if let Some(n) = label.strip_prefix('v').and_then(|s| s.parse::<i64>().ok()) {
        n
    } else {
        -1
    }
}

fn mtime_secs(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Compute the next version number for a multi-version skill (max existing vN + 1).
pub fn next_version_number(skill_root: &Path) -> u32 {
    let mut max: u32 = 0;
    if let Ok(entries) = fs::read_dir(skill_root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(n) = name.strip_prefix('v').and_then(|s| s.parse::<u32>().ok()) {
                if n > max {
                    max = n;
                }
            }
        }
    }
    max + 1
}

/// Snapshot current `latest` into a new `v<next>` and return the new label.
/// Promotes a flat-layout skill to multi-version layout as a side effect:
///   `<root>/`          →   `<root>/latest/`  +  `<root>/v<N>/`
/// Both copies start identical; `latest` stays mutable, `v<N>` is the snapshot.
/// If `note` is Some(non-empty), it is written as a `.version-note` sidecar
/// inside the new snapshot directory (PRD §4.5c, DECISIONS 14).
pub fn snapshot_version(skill_root: &Path) -> Result<String> {
    snapshot_version_with_note(skill_root, None)
}

/// Remove legacy flat-layout entries from `skill_root` after it has been
/// promoted to multi-version layout. Keeps `latest/` and all `v<N>/` dirs.
fn cleanup_flat_root(skill_root: &Path) -> Result<()> {
    for entry in fs::read_dir(skill_root)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == LATEST_VERSION {
            continue;
        }
        if name_str.len() > 1 {
            if let Some(rest) = name_str.strip_prefix('v') {
                if rest.parse::<u32>().is_ok() {
                    continue;
                }
            }
        }
        remove_path(&entry.path())?;
    }
    Ok(())
}

/// Same as [`snapshot_version`] but attaches an optional human note.
///
/// Uses copy-in semantics: the current content is first copied into a staging
/// area, then the immutable `v<N>/` snapshot is created from that staging copy.
/// Only after both copies succeed does an atomic rename promote staging to
/// `latest/`. A crash at any point leaves the original skill root intact.
pub fn snapshot_version_with_note(skill_root: &Path, note: Option<&str>) -> Result<String> {
    let latest_dir = skill_root.join(LATEST_VERSION);
    let was_flat = !latest_dir.is_dir();

    // Stage the source content we will snapshot from.
    let (source_dir, staging) = if was_flat {
        // Copy the flat root into a sibling staging directory. We never modify
        // skill_root until the final atomic rename succeeds.
        let s = skill_root.with_extension("skillmint-latest-staging");
        if s.exists() || s.is_symlink() {
            remove_path(&s)?;
        }
        copy_dir_all(skill_root, &s)?;
        (s.clone(), Some(s))
    } else {
        (latest_dir.clone(), None)
    };

    let result: Result<String> = (|| {
        let n = next_version_number(skill_root);
        let label = format!("v{}", n);
        let dest = skill_root.join(&label);
        copy_dir_all(&source_dir, &dest)?;
        write_version_note(&dest, note)?;

        if was_flat {
            // Atomically promote the staging copy to latest/.
            replace_path_atomic(&source_dir, &latest_dir)?;
            // Now that a safe latest/ exists, remove original flat entries.
            cleanup_flat_root(skill_root)?;
        }
        Ok(label)
    })();

    if result.is_err() {
        // Clean up the sibling staging copy on failure. After a successful
        // replace_path_atomic the staging path no longer exists, so this is a no-op.
        if let Some(ref s) = staging {
            if s.exists() || s.is_symlink() {
                let _ = remove_path(s);
            }
        }
    }
    result
}

// =============================================================================
// PRD-0 §4.8: Center Repo zip backup / restore
// =============================================================================

use std::fs::File;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// Pack `src` (the center repo directory) into a zip at `dst`.
/// Each top-level entry under `src` is a Skill directory; files are stored with
/// paths relative to `src`. Hidden files (`.DS_Store`) and the `.skillmint`
/// metadata dir are skipped. PRD-0 §4.8.
pub fn zip_dir(src: &Path, dst: &Path) -> Result<()> {
    let file = File::create(dst).context("failed to create zip file")?;
    let mut zip = ZipWriter::new(file);
    let options =
        SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    for entry in WalkDir::new(src)
        .into_iter()
        .filter_entry(|e| {
            // Skip excluded names anywhere in the tree.
            let name = e.file_name().to_string_lossy();
            !(name == ".DS_Store" || name == ".skillmint")
        })
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        // Don't add the root itself.
        if path == src {
            continue;
        }
        let rel = match path.strip_prefix(src) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");

        if entry.file_type().is_dir() {
            // Record directory entries so empty skill dirs survive round-trip.
            zip.add_directory(&rel_str, options)?;
        } else if entry.file_type().is_file() {
            zip.start_file(&rel_str, options)?;
            let mut f = File::open(path)?;
            io::copy(&mut f, &mut zip)?;
        }
    }

    zip.finish().context("failed to finalize zip")?;
    Ok(())
}

/// Unpack a backup zip into `dest_root`, returning the list of top-level
/// (skill) directory names written. Does NOT merge — the caller decides how to
/// reconcile with the existing center repo (see `restore_center_repo` command).
pub fn unzip_to(zip_path: &Path, dest_root: &Path) -> Result<Vec<String>> {
    let file = File::open(zip_path).context("failed to open zip file")?;
    let mut archive = ZipArchive::new(file)?;

    fs::create_dir_all(dest_root)?;
    let mut top_level: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let outpath = match entry.enclosed_name() {
            Some(p) => dest_root.join(p),
            None => continue, // skip unsafe paths (zip-slip guard)
        };
        let name = entry.name().to_string();
        if name.ends_with('/') {
            // Directory entry.
            fs::create_dir_all(&outpath)?;
        } else {
            if let Some(p) = outpath.parent() {
                fs::create_dir_all(p)?;
            }
            let mut outfile = File::create(&outpath)?;
            io::copy(&mut entry, &mut outfile)?;
        }
        // Track the top-level segment (the skill name).
        let top_name = entry
            .enclosed_name()
            .and_then(|p| p.iter().next().map(|s| s.to_owned()));
        if let Some(first) = top_name {
            top_level.insert(first.to_string_lossy().to_string());
        }
    }

    Ok(top_level.into_iter().collect())
}

/// Move every top-level skill dir from `src_root` into `center_repo` that does
/// NOT already exist there (smart-merge: never overwrite). Returns
/// `(imported_names, skipped_names)`. PRD-0 §4.8 import (restore) semantics.
pub fn merge_into_center(
    src_root: &Path,
    center_repo: &Path,
) -> Result<(Vec<String>, Vec<String>)> {
    let mut imported = Vec::new();
    let mut skipped = Vec::new();
    fs::create_dir_all(center_repo)?;
    for entry in fs::read_dir(src_root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.is_empty() {
            continue;
        }
        let dest = center_repo.join(&name);
        if dest.exists() {
            // Smart merge: do not overwrite existing skills.
            skipped.push(name);
            continue;
        }
        // Try atomic rename first; fall back to copy+remove for cross-volume merges.
        if let Err(e) = fs::rename(&path, &dest) {
            let is_cross_device = e.kind() == io::ErrorKind::CrossesDevices
                || e.to_string().to_lowercase().contains("cross-device")
                || cfg!(unix) && e.raw_os_error() == Some(18); // EXDEV
            if is_cross_device {
                copy_dir_all(&path, &dest)?;
                let _ = remove_path(&path);
            } else {
                return Err(e.into());
            }
        }
        imported.push(name);
    }
    Ok((imported, skipped))
}

