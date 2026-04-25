use chrono::Utc;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use vidlib_core::{
    ActionStatus, AuditRecord, AuditRecordKind, RestructureAction, RestructurePlan, UndoManifest,
    VidLibError, VidLibResult, VideoEntry,
};

pub fn plan_by_extension(entries: &[VideoEntry], destination_root: &Path) -> RestructurePlan {
    let mut reserved = HashSet::<PathBuf>::new();
    let actions = entries
        .iter()
        .map(|entry| {
            let extension = entry.extension.clone().unwrap_or_else(|| "unknown".into());
            let target_dir = destination_root.join(extension);
            let destination = target_dir.join(&entry.file_name);
            let conflict = if !reserved.insert(destination.clone()) || destination.exists() {
                Some("destination already exists".into())
            } else {
                None
            };
            RestructureAction {
                source: entry.path.clone(),
                destination,
                conflict,
                status: ActionStatus::Pending,
            }
        })
        .collect();

    RestructurePlan {
        id: Uuid::new_v4(),
        created_at: Utc::now(),
        actions,
        dry_run: true,
    }
}

pub fn apply_plan(
    plan: &RestructurePlan,
    manifest_path: &Path,
    force: bool,
) -> VidLibResult<UndoManifest> {
    if plan.dry_run && !force {
        return Err(VidLibError::Validation(
            "plan is marked dry-run; re-run with explicit confirmation".into(),
        ));
    }
    if plan.actions.iter().any(|action| action.conflict.is_some()) {
        return Err(VidLibError::FileOps(
            "plan contains conflicts; resolve before applying".into(),
        ));
    }

    let mut reverse_actions = Vec::with_capacity(plan.actions.len());
    let mut audit_log = Vec::new();
    for action in &plan.actions {
        if let Some(parent) = action.destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| VidLibError::FileOps(format!("creating {}: {err}", parent.display())))?;
        }
        if !action.source.exists() {
            return Err(VidLibError::FileOps(format!("source missing: {}", action.source.display())));
        }
        if action.destination.exists() {
            return Err(VidLibError::FileOps(format!("destination exists: {}", action.destination.display())));
        }
        fs::rename(&action.source, &action.destination).map_err(|err| {
            VidLibError::FileOps(format!(
                "moving {} -> {}: {err}",
                action.source.display(),
                action.destination.display()
            ))
        })?;
        reverse_actions.push(RestructureAction {
            source: action.destination.clone(),
            destination: action.source.clone(),
            conflict: None,
            status: ActionStatus::Applied,
        });
        audit_log.push(format!(
            "applied: {} -> {}",
            action.source.display(),
            action.destination.display()
        ));
    }

    let manifest = UndoManifest {
        plan_id: plan.id,
        generated_at: Utc::now(),
        reverse_actions,
        audit_log,
    };
    fs::write(manifest_path, serde_json::to_vec_pretty(&manifest)?)
        .map_err(|err| VidLibError::FileOps(format!("writing {}: {err}", manifest_path.display())))?;
    Ok(manifest)
}

pub fn undo_from_manifest(manifest: &UndoManifest) -> VidLibResult<()> {
    for action in &manifest.reverse_actions {
        if let Some(parent) = action.destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| VidLibError::FileOps(format!("creating {}: {err}", parent.display())))?;
        }
        if !action.source.exists() {
            continue;
        }
        fs::rename(&action.source, &action.destination).map_err(|err| {
            VidLibError::FileOps(format!(
                "restoring {} -> {}: {err}",
                action.source.display(),
                action.destination.display()
            ))
        })?;
    }
    Ok(())
}

pub fn audit_records_for_manifest(manifest: &UndoManifest) -> Vec<AuditRecord> {
    let mut records = Vec::new();
    records.push(AuditRecord {
        id: Uuid::new_v4(),
        plan_id: Some(manifest.plan_id),
        kind: AuditRecordKind::PlanApplied,
        source_path: None,
        destination_path: None,
        details: format!("plan {} applied", manifest.plan_id),
        created_at: manifest.generated_at,
    });
    for action in &manifest.reverse_actions {
        records.push(AuditRecord {
            id: Uuid::new_v4(),
            plan_id: Some(manifest.plan_id),
            kind: AuditRecordKind::FileMoved,
            source_path: Some(action.destination.clone()),
            destination_path: Some(action.source.clone()),
            details: format!(
                "move recorded: {} -> {}",
                action.destination.display(),
                action.source.display()
            ),
            created_at: manifest.generated_at,
        });
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_entry(path: PathBuf) -> VideoEntry {
        VideoEntry {
            id: Uuid::new_v4(),
            file_name: path.file_name().unwrap().to_string_lossy().into_owned(),
            extension: Some("mp4".into()),
            path,
            size_bytes: 1,
            modified_at: None,
            duration_seconds: None,
            width: None,
            height: None,
            codec: None,
            exact_hash: None,
            chunk_hash: None,
            perceptual_hash: None,
            tags: Vec::new(),
        }
    }

    #[test]
    fn plans_and_undoes_moves() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.mp4");
        fs::write(&src, b"test").unwrap();
        let entry = sample_entry(src.clone());
        let plan = plan_by_extension(&[entry], &dir.path().join("organized"));
        assert!(plan.actions[0].conflict.is_none());

        let manifest_path = dir.path().join("undo.json");
        let mut plan = plan;
        plan.dry_run = false;
        let manifest = apply_plan(&plan, &manifest_path, true).unwrap();
        assert!(!src.exists());
        assert!(plan.actions[0].destination.exists());

        undo_from_manifest(&manifest).unwrap();
        assert!(src.exists());
    }

    #[test]
    fn rejects_dry_run_apply_without_force() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.mp4");
        fs::write(&src, b"test").unwrap();
        let entry = sample_entry(src);
        let plan = plan_by_extension(&[entry], &dir.path().join("organized"));
        let result = apply_plan(&plan, &dir.path().join("undo.json"), false);
        assert!(result.is_err());
    }
}
