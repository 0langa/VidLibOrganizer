use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use vidlib_engine::{ScanProgress, ScanResult};
use vidlib_fileops::{
    coordinator::{Coordinator, Epochs, Lease, LeaseKind},
    ErrorCode, OpError, Phase,
};

#[derive(Clone, Serialize)]
pub struct Status {
    pub id: Option<String>,
    pub running: bool,
    pub progress: ScanProgress,
    pub error: Option<String>,
    pub result_ready: bool,
    pub acting: bool,
}

pub struct Jobs {
    coordinator: Arc<Coordinator>,
    result_epochs: Option<Epochs>,
    pub id: Option<String>,
    pub running: bool,
    pub acting: bool,
    pub progress: ScanProgress,
    pub error: Option<String>,
    pub result: Option<ScanResult>,
    pub cancel: Arc<AtomicBool>,
    sequence: u64,
}

impl Jobs {
    pub fn new(coordinator: Arc<Coordinator>) -> Self {
        Self {
            coordinator,
            result_epochs: None,
            id: None,
            running: false,
            acting: false,
            progress: ScanProgress::default(),
            error: None,
            result: None,
            cancel: Arc::new(AtomicBool::new(false)),
            sequence: 0,
        }
    }

    pub fn begin(&mut self) -> Result<(String, Arc<AtomicBool>, Lease), OpError> {
        if self.running || self.acting {
            return Err(OpError::new(
                ErrorCode::Busy,
                Phase::Read,
                "Wait for the current scan or file operation to finish.",
            ));
        }
        let lease = self.coordinator.try_acquire(LeaseKind::Read)?;
        self.result_epochs = Some(lease.epochs()?);
        self.sequence += 1;
        let id = self.sequence.to_string();
        self.id = Some(id.clone());
        self.cancel = Arc::new(AtomicBool::new(false));
        self.running = true;
        self.result = None;
        self.error = None;
        self.progress = ScanProgress {
            phase: "discovering".into(),
            ..Default::default()
        };
        Ok((id, self.cancel.clone(), lease))
    }

    pub fn cancel(&self, id: &str) -> Result<(), String> {
        if self.id.as_deref() != Some(id) {
            return Err("That scan is no longer current.".into());
        }
        self.cancel.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub fn finish(&mut self, outcome: Result<ScanResult, String>) {
        self.running = false;
        match outcome {
            Ok(result) => {
                self.progress.phase = if result.cancelled {
                    "cancelled"
                } else {
                    "complete"
                }
                .into();
                self.result = Some(result);
            }
            Err(error) => {
                self.progress.phase = "failed".into();
                self.error = Some(error);
            }
        }
    }

    pub fn result(&self, id: &str) -> Result<&ScanResult, String> {
        if self.id.as_deref() != Some(id) {
            return Err("That scan is no longer current.".into());
        }
        self.result
            .as_ref()
            .ok_or_else(|| "Scan results are not ready yet.".into())
    }

    pub fn status(&self) -> Status {
        Status {
            id: self.id.clone(),
            running: self.running,
            progress: self.progress.clone(),
            error: self.error.clone(),
            result_ready: self.result.is_some(),
            acting: self.acting,
        }
    }

    pub fn begin_action(&mut self, id: &str) -> Result<(ScanResult, Lease), OpError> {
        if self.running || self.acting {
            return Err(OpError::new(
                ErrorCode::Busy,
                Phase::Write,
                "Another operation is still running.",
            ));
        }
        let lease = self.coordinator.try_acquire(LeaseKind::Write)?;
        let epochs = self.result_epochs.ok_or_else(|| {
            OpError::new(
                ErrorCode::StaleState,
                Phase::Verify,
                "Scan results are no longer current.",
            )
        })?;
        lease.verify_epochs(epochs)?;
        let result = self
            .result(id)
            .map_err(|e| OpError::new(ErrorCode::StaleState, Phase::Verify, e))?
            .clone();
        self.acting = true;
        Ok((result, lease))
    }

    pub fn finish_action_result(
        &mut self,
        outcome: &Result<vidlib_fileops::ActionReport, OpError>,
        lease: &Lease,
    ) {
        let completed = outcome
            .as_ref()
            .map(|report| report.completed.as_slice())
            .unwrap_or_default();
        let epochs = lease
            .status()
            .ok()
            .filter(|status| !status.requires_recovery())
            .map(|status| status.epochs);
        self.finish_action(completed, epochs);
        match outcome {
            Ok(report) => {
                self.error = None;
                if let Some(result) = &mut self.result {
                    result.issues.extend(report.issues.iter().cloned());
                }
            }
            Err(error) => self.error = Some(error.message.clone()),
        }
    }
    pub fn finish_action(&mut self, completed: &[String], epochs: Option<Epochs>) {
        self.result_epochs = epochs;
        if let Some(result) = &mut self.result {
            for group in &mut result.groups {
                group.files.retain(|file| !completed.contains(&file.path));
                group.redundant_bytes = group
                    .size
                    .saturating_mul(group.files.len().saturating_sub(1) as u64);
            }
            result.groups.retain(|group| group.files.len() > 1);
        }
        self.acting = false;
    }
}

/// This transitional marker only records batch uncertainty. Task27 adds per-item facts.
pub fn run_action(
    lease: &Lease,
    result: &ScanResult,
    paths: &[String],
    action: vidlib_fileops::FileAction,
    destination: Option<&str>,
) -> Result<vidlib_fileops::ActionReport, OpError> {
    let mutation = lease.begin_legacy_mutation()?;
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        vidlib_fileops::apply_actions(result, paths, action, destination)
    }));
    match outcome {
        Ok(result) => {
            mutation.finish()?;
            result.map_err(|e| OpError::new(ErrorCode::Io, Phase::Write, e))
        }
        Err(_) => Err(OpError::new(
            ErrorCode::RecoveryRequired,
            Phase::Write,
            "The file operation stopped unexpectedly. Recovery is required.",
        )),
    }
}

/// Bounded Rename job progress and owned completion facts. Cancellation only
/// requests a stop; a factual partial-operation result is still delivered.
pub(crate) struct RenameJob {
    pub control: vidlib_inventory::JobControl,
    pub status: crate::commands::rename::JobStatusDto,
    progress: std::sync::mpsc::Receiver<vidlib_inventory::Progress>,
}
impl RenameJob {
    pub fn new() -> Self {
        let (control, progress) = vidlib_inventory::JobControl::with_progress(4);
        Self {
            control,
            progress,
            status: crate::commands::rename::JobStatusDto {
                state: "running".into(),
                result: None,
                error: None,
                progress: None,
            },
        }
    }
    pub fn status(&mut self) -> crate::commands::rename::JobStatusDto {
        if let Some(progress) = self.progress.try_iter().last() {
            self.status.progress = Some(serde_json::json!({
                "phase": match progress.phase { vidlib_inventory::ProgressPhase::Scanning => "scanning", vidlib_inventory::ProgressPhase::Selecting => "selecting", vidlib_inventory::ProgressPhase::Evaluating => "evaluating", vidlib_inventory::ProgressPhase::Complete => "complete", vidlib_inventory::ProgressPhase::Cancelled => "cancelled" },
                "done": progress.done.to_string(), "total": progress.total.map(|n| n.to_string()), "currentEntry": progress.current_entry,
            }));
        }
        self.status.clone()
    }
    pub fn finish(
        &mut self,
        outcome: Result<serde_json::Value, crate::commands::rename::CommandError>,
    ) {
        match outcome {
            Ok(result) => {
                self.status.state = "completed".into();
                self.status.result = Some(result);
            }
            Err(error) => {
                self.status.state = if self.control.is_cancelled() {
                    "cancelled"
                } else {
                    "failed"
                }
                .into();
                self.status.error = Some(error);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vidlib_engine::{DuplicateGroup, ScanMode, VideoFile};

    fn result() -> ScanResult {
        ScanResult {
            mode: ScanMode::Size,
            groups: vec![DuplicateGroup {
                id: "g".into(),
                size: 8,
                hash: None,
                redundant_bytes: 16,
                files: ["a", "b", "c"]
                    .map(|path| VideoFile {
                        path: path.into(),
                        name: path.into(),
                        size: 8,
                        modified_ms: 0,
                    })
                    .to_vec(),
            }],
            issues: vec![],
            files_scanned: 3,
            bytes_scanned: 24,
            elapsed_ms: 1,
            cancelled: false,
        }
    }

    fn coordinated_jobs(coordinator: Arc<Coordinator>) -> Jobs {
        Jobs::new(coordinator)
    }

    fn fixture_jobs() -> (tempfile::TempDir, Jobs) {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Jobs::new(Arc::new(Coordinator::open(dir.path()).unwrap()));
        (dir, jobs)
    }

    // Ordinary validation rejection must not force a rescan before correcting selection.
    #[test]
    fn known_validation_error_preserves_actionable_cache_and_status_message() {
        let (_dir, mut jobs) = fixture_jobs();
        let id = jobs.begin().unwrap().0;
        jobs.finish(Ok(result()));
        let (scan, lease) = jobs.begin_action(&id).unwrap();
        let outcome = run_action(&lease, &scan, &[], vidlib_fileops::FileAction::Delete, None);
        assert!(outcome.is_err());
        jobs.finish_action_result(&outcome, &lease);
        assert_eq!(
            jobs.status().error.as_deref(),
            Some("Select at least one copy.")
        );
        drop(lease);
        assert!(
            jobs.begin_action(&id).is_ok(),
            "ordinary rejected selection must remain correctable without a rescan"
        );
    }

    // Known partial success must advance the epoch and retain authority for remaining copies.
    #[test]
    fn known_partial_action_keeps_remaining_duplicates_actionable() {
        use vidlib_fileops::{coordinator::LeaseKind, FileAction};
        let (dir, mut jobs) = fixture_jobs();
        let media = dir.path().join("media");
        let destination = dir.path().join("destination");
        std::fs::create_dir(&media).unwrap();
        std::fs::create_dir(&destination).unwrap();
        let blocked_name = format!("{}.mp4", "b".repeat(251));
        for name in ["a.mp4", blocked_name.as_str(), "c.mp4"] {
            std::fs::write(media.join(name), b"same media").unwrap();
        }
        std::fs::write(destination.join(&blocked_name), b"must not replace").unwrap();
        let (id, cancel, scan_lease) = jobs.begin().unwrap();
        let scan = vidlib_engine::scan(
            vec![media.to_string_lossy().into_owned()],
            ScanMode::Md5,
            cancel,
            |_| {},
        )
        .unwrap();
        jobs.finish(Ok(scan));
        drop(scan_lease);
        let (scan, lease) = jobs.begin_action(&id).unwrap();
        let chosen = [
            media.join(&blocked_name).to_string_lossy().into_owned(),
            media.join("c.mp4").to_string_lossy().into_owned(),
        ];
        let report = run_action(
            &lease,
            &scan,
            &chosen,
            FileAction::Move,
            destination.to_str(),
        )
        .unwrap();
        assert_eq!(report.completed, vec![chosen[1].clone()]);
        assert!(!report.issues.is_empty());
        assert_eq!(
            std::fs::read(destination.join(&blocked_name)).unwrap(),
            b"must not replace"
        );
        assert!(!media.join("c.mp4").exists());
        assert_eq!(lease.epochs().unwrap().namespace, 1);
        jobs.finish_action_result(&Ok(report), &lease);
        drop(lease);
        assert!(
            jobs.begin_action(&id).is_ok(),
            "known own mutation must refresh remaining duplicate authority"
        );
        assert!(jobs.coordinator.try_acquire(LeaseKind::Read).is_ok());
    }

    #[test]
    fn duplicate_scan_uses_shared_coordinator() {
        use vidlib_fileops::coordinator::{Coordinator, LeaseKind};
        let dir = tempfile::tempdir().unwrap();
        let coordinator = Arc::new(Coordinator::open(dir.path()).unwrap());
        let mut jobs = coordinated_jobs(coordinator.clone());
        let _writer = coordinator.try_acquire(LeaseKind::Write).unwrap();
        assert!(
            jobs.begin().is_err(),
            "duplicate scan bypassed exclusive writer"
        );
    }

    // Duplicate deletion/move must contend with every physical collection reader.
    #[test]
    fn duplicate_actions_use_shared_coordinator() {
        use vidlib_fileops::coordinator::{Coordinator, LeaseKind};
        let dir = tempfile::tempdir().unwrap();
        let coordinator = Arc::new(Coordinator::open(dir.path()).unwrap());
        let mut jobs = coordinated_jobs(coordinator.clone());
        let id = jobs.begin().unwrap().0;
        jobs.finish(Ok(result()));
        let _reader = coordinator.try_acquire(LeaseKind::Read).unwrap();
        assert!(
            jobs.begin_action(&id).is_err(),
            "duplicate action bypassed active reader"
        );
    }

    // Cached browsing remains available, while a foreign mutation revokes action authority.
    #[test]
    fn duplicate_cache_is_browsable_but_old_epoch_cannot_mutate() {
        use vidlib_fileops::coordinator::{Coordinator, LeaseKind};
        let dir = tempfile::tempdir().unwrap();
        let coordinator = Arc::new(Coordinator::open(dir.path()).unwrap());
        let mut jobs = coordinated_jobs(coordinator.clone());
        let id = jobs.begin().unwrap().0;
        jobs.finish(Ok(result()));
        Coordinator::open(dir.path())
            .unwrap()
            .try_acquire(LeaseKind::Write)
            .unwrap()
            .advance_namespace()
            .unwrap();
        assert_eq!(jobs.result(&id).unwrap().groups.len(), 1);
        assert!(
            jobs.begin_action(&id).is_err(),
            "stale duplicate results retained action authority"
        );
    }

    #[test]
    fn active_scan_rejects_second_start_and_cancellation_reaches_worker() {
        let (_dir, mut jobs) = fixture_jobs();
        let (id, token, lease) = jobs.begin().unwrap();
        assert!(jobs.begin().is_err());
        assert!(jobs.cancel("stale").is_err());
        assert!(!token.load(Ordering::Relaxed));
        jobs.cancel(&id).unwrap();
        assert!(token.load(Ordering::Relaxed));
        jobs.finish(Ok(result()));
        drop(lease);
        let (_, new_token, _lease) = jobs.begin().unwrap();
        assert!(!new_token.load(Ordering::Relaxed));
        assert!(jobs.result(&id).is_err());
    }

    #[test]
    fn action_reserves_job_and_removes_only_completed_files() {
        let (_dir, mut jobs) = fixture_jobs();
        let (id, _, lease) = jobs.begin().unwrap();
        drop(lease);
        jobs.finish(Ok(result()));
        jobs.begin_action(&id).unwrap();
        assert!(jobs.begin().is_err());
        assert!(jobs.begin_action(&id).is_err());
        jobs.finish_action(&["b".into()], jobs.result_epochs);
        let remaining = &jobs.result(&id).unwrap().groups[0];
        assert_eq!(remaining.files.len(), 2);
        assert_eq!(remaining.redundant_bytes, 8);
        assert_eq!(remaining.files[1].path, "c");
        jobs.finish_action(&["c".into()], jobs.result_epochs);
        assert!(jobs.result(&id).unwrap().groups.is_empty());
    }

    #[test]
    fn failed_scan_reports_error_and_releases_slot() {
        let (_dir, mut jobs) = fixture_jobs();
        jobs.begin().unwrap();
        jobs.finish(Err("Drive disconnected".into()));
        assert!(!jobs.status().running);
        assert!(!jobs.status().result_ready);
        assert_eq!(jobs.status().error.as_deref(), Some("Drive disconnected"));
        assert!(jobs.begin().is_ok());
    }
}
