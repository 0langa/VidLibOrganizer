use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use vidlib_engine::{ScanProgress, ScanResult};

#[derive(Clone, Serialize)]
pub struct Status {
    pub id: Option<String>,
    pub running: bool,
    pub progress: ScanProgress,
    pub error: Option<String>,
    pub result_ready: bool,
    pub acting: bool,
}

#[derive(Default)]
pub struct Jobs {
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
    pub fn begin(&mut self) -> Result<(String, Arc<AtomicBool>), String> {
        if self.running || self.acting {
            return Err("Wait for the current scan or file operation to finish.".into());
        }
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
        Ok((id, self.cancel.clone()))
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

    pub fn begin_action(&mut self, id: &str) -> Result<ScanResult, String> {
        if self.running || self.acting {
            return Err("Another operation is still running.".into());
        }
        let result = self.result(id)?.clone();
        self.acting = true;
        Ok(result)
    }

    pub fn finish_action(&mut self, completed: &[String]) {
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

    #[test]
    fn active_scan_rejects_second_start_and_cancellation_reaches_worker() {
        let mut jobs = Jobs::default();
        let (id, token) = jobs.begin().unwrap();
        assert!(jobs.begin().is_err());
        assert!(jobs.cancel("stale").is_err());
        assert!(!token.load(Ordering::Relaxed));
        jobs.cancel(&id).unwrap();
        assert!(token.load(Ordering::Relaxed));
        jobs.finish(Ok(result()));
        let (_, new_token) = jobs.begin().unwrap();
        assert!(!new_token.load(Ordering::Relaxed));
        assert!(jobs.result(&id).is_err());
    }

    #[test]
    fn action_reserves_job_and_removes_only_completed_files() {
        let mut jobs = Jobs::default();
        let (id, _) = jobs.begin().unwrap();
        jobs.finish(Ok(result()));
        jobs.begin_action(&id).unwrap();
        assert!(jobs.begin().is_err());
        assert!(jobs.begin_action(&id).is_err());
        jobs.finish_action(&["b".into()]);
        let remaining = &jobs.result(&id).unwrap().groups[0];
        assert_eq!(remaining.files.len(), 2);
        assert_eq!(remaining.redundant_bytes, 8);
        assert_eq!(remaining.files[1].path, "c");
        jobs.finish_action(&["c".into()]);
        assert!(jobs.result(&id).unwrap().groups.is_empty());
    }

    #[test]
    fn failed_scan_reports_error_and_releases_slot() {
        let mut jobs = Jobs::default();
        jobs.begin().unwrap();
        jobs.finish(Err("Drive disconnected".into()));
        assert!(!jobs.status().running);
        assert!(!jobs.status().result_ready);
        assert_eq!(jobs.status().error.as_deref(), Some("Drive disconnected"));
        assert!(jobs.begin().is_ok());
    }
}
