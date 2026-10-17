use std::{
    fs,
    fs::File,
    path::{Path, PathBuf},
};
use vidlib_fileops::{
    coordinator::{Coordinator, LeaseKind},
    journal::Journal,
    platform::{Durability, MetadataUpdate, MutationOutcome, OwnedEntry, StagingFile},
    *,
};
use vidlib_inventory::{ids::*, *};
use vidlib_rename::*;

pub struct Fixture {
    _dir: tempfile::TempDir,
    pub media: PathBuf,
    pub coordinator: Coordinator,
    pub plan: RenamePlan,
}
impl Fixture {
    pub fn new(names: &[(&str, &str)]) -> Self {
        let dir = if let Ok(root) = std::env::var("VIDLIB_CRASH_ROOT") {
            tempfile::Builder::new()
                .prefix("fixture-")
                .tempdir_in(root)
                .unwrap()
        } else {
            tempfile::tempdir().unwrap()
        };
        Self::at(dir, names)
    }
    pub fn at(dir: tempfile::TempDir, names: &[(&str, &str)]) -> Self {
        let media = dir.path().join("media");
        fs::create_dir(&media).unwrap();
        for (name, _) in names {
            fs::write(
                media.join(name),
                if *name == "a" {
                    b"fixture".as_slice()
                } else {
                    name.as_bytes()
                },
            )
            .unwrap();
        }
        let coordinator = Coordinator::open(&dir.path().join("app")).unwrap();
        let inventory = scan(
            &InventoryRequest::new(vec![RootBinding::new(media.clone()).unwrap()]),
            &JobControl::default(),
        )
        .unwrap();
        let selection: Vec<_> = names
            .iter()
            .map(|(name, _)| {
                inventory
                    .entries
                    .iter()
                    .find(|e| e.absolute_path == media.join(name))
                    .unwrap()
                    .id
            })
            .collect();
        let recipe = RecipeV1 {
            schema_version: 1,
            revision: 1,
            selection: selection.clone(),
            selection_filter: None,
            numbering_order: selection.clone(),
            numbering_scope: Default::default(),
            rules: vec![],
            companions: CompanionPolicy::None,
            destination: Destination::SameDirectory,
            collision_policy: CollisionPolicy::Block,
            metadata_changes: vec![],
        };
        let frozen = FrozenInputs {
            inventory_id: inventory.id,
            generation: inventory.generation,
            now_ns: 0,
            timezone: FrozenTimezone::Utc,
            primary_order: selection,
            numbering_scope: Default::default(),
            values: vec![],
        };
        let mut batch = evaluate(&inventory, &recipe, &frozen).unwrap();
        for row in &mut batch.rows {
            let (_, target) = names
                .iter()
                .find(|(name, _)| row.source_path == media.join(name))
                .unwrap();
            row.target_path = media.join(target);
            row.status = RowStatus::Changed;
        }
        let namespace = PlatformFs::default()
            .capture_namespace(
                &batch.grants,
                &batch
                    .rows
                    .iter()
                    .map(|r| r.source_path.clone())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let plan = plan(
            &inventory,
            &batch,
            &namespace,
            PlanRef {
                session_id: SessionId::new(),
                plan_id: PlanId::new(),
                revision: 1,
            },
        )
        .unwrap();
        assert_eq!(plan.counts.blocked, 0, "{:?}", plan.issues);
        Self {
            _dir: dir,
            media,
            coordinator,
            plan,
        }
    }
    pub fn run(
        &self,
        adapter: &mut dyn Filesystem,
        control: &JobControl,
    ) -> Result<OperationReport, OpError> {
        let lease = self.coordinator.try_acquire(LeaseKind::Write).unwrap();
        let mut journal = Journal::open(&self.coordinator.database_path()).unwrap();
        execute(&self.plan, &lease, &mut journal, adapter, control)
    }
}
pub struct CrashFs {
    pub outcome_durability: Option<Durability>,
    pub after_capture: Option<PathBuf>,
    pub native: PlatformFs,
    pub boundary: String,
    pub index: usize,
    pub count: usize,
}
impl CrashFs {
    fn before(&mut self) {
        self.count += 1;
        if self.count == self.index && self.boundary == "before" {
            std::process::exit(86);
        }
    }
    fn after(&self) {
        if self.count == self.index && self.boundary == "after" {
            std::process::exit(86);
        }
    }
}
impl Filesystem for CrashFs {
    fn rename_reviewed(
        &mut self,
        r: &platform::RenameRequest<'_>,
    ) -> Result<MutationOutcome, OpError> {
        self.before();
        let mut out = self.native.rename_reviewed(r)?;
        if let Some(d) = &self.outcome_durability {
            out.durability = d.clone();
        }
        self.after();
        Ok(out)
    }
    fn rename_reserved(
        &mut self,
        r: &platform::RenameReservation,
    ) -> Result<MutationOutcome, OpError> {
        self.before();
        let mut out = self.native.rename_reserved(r)?;
        if let Some(d) = &self.outcome_durability {
            out.durability = d.clone();
        }
        self.after();
        Ok(out)
    }
    fn capture_namespace(&self, g: &GrantSet, c: &[PathBuf]) -> Result<NamespaceSnapshot, OpError> {
        let captured = self.native.capture_namespace(g, c)?;
        if let Some(path) = &self.after_capture {
            fs::write(path, b"unowned intruder").unwrap();
        }
        Ok(captured)
    }
    fn inspect(&self, p: &Path) -> Result<FileObservation, OpError> {
        if self.boundary == "receipt" && self.count == self.index {
            // The executor has committed the preceding receipt, but has not
            // written the next intent when it inspects that step's parents.
            std::process::exit(86);
        }
        if let Some(database) = self.boundary.strip_prefix("post_reset:") {
            let db = rusqlite::Connection::open(database).unwrap();
            let reset: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM recovery_audit a JOIN steps s ON s.operation_id=a.operation_id AND s.id=a.step_id WHERE a.resolution='NotApplied' AND s.state='Planned')", [], |r|r.get(0)).unwrap();
            if reset {
                std::process::exit(86);
            }
        }
        if let Some(database) = self.boundary.strip_prefix("resolved_gap:") {
            let db = rusqlite::Connection::open(database).unwrap();
            let resolved: bool = db.query_row("SELECT NOT EXISTS(SELECT 1 FROM steps WHERE state!='CompletionDurable') AND NOT EXISTS(SELECT 1 FROM recovery_effects)", [], |r|r.get(0)).unwrap();
            if resolved {
                std::process::exit(86);
            }
        }
        self.native.inspect(p)
    }
    fn verify(&self, p: &Path, s: &FileStamp) -> Result<FileObservation, OpError> {
        self.native.verify(p, s)
    }
    fn rename_no_replace(
        &mut self,
        s: &Path,
        t: &Path,
        e: &FileStamp,
    ) -> Result<MutationOutcome, OpError> {
        self.native.rename_no_replace(s, t, e)
    }
    fn publish_staging(&mut self, o: &OwnedEntry, t: &Path) -> Result<MutationOutcome, OpError> {
        self.native.publish_staging(o, t)
    }
    fn create_directory(&mut self, p: &Path) -> Result<OwnedEntry, OpError> {
        self.native.create_directory(p)
    }
    fn create_staging(&mut self, p: &Path) -> Result<StagingFile, OpError> {
        self.native.create_staging(p)
    }
    fn refresh_staging(&self, s: &mut StagingFile) -> Result<Durability, OpError> {
        self.native.refresh_staging(s)
    }
    fn open_verified(&self, p: &Path, s: &FileStamp) -> Result<File, OpError> {
        self.native.open_verified(p, s)
    }
    fn set_metadata(
        &mut self,
        p: &Path,
        s: &FileStamp,
        u: &MetadataUpdate,
    ) -> Result<MutationOutcome, OpError> {
        self.native.set_metadata(p, s, u)
    }
    fn flush_directory(&self, p: &Path) -> Result<Durability, OpError> {
        self.native.flush_directory(p)
    }
    fn remove_owned(&mut self, o: &OwnedEntry) -> Result<Durability, OpError> {
        self.native.remove_owned(o)
    }
}

pub fn replan(f: &mut Fixture, root: PathBuf, mapping: &[(&str, &str)]) {
    let inventory = scan(
        &InventoryRequest::new(vec![RootBinding::new(root).unwrap()]),
        &JobControl::default(),
    )
    .unwrap();
    let selection: Vec<_> = mapping
        .iter()
        .map(|(s, _)| {
            inventory
                .entries
                .iter()
                .find(|e| e.absolute_path == f.media.join(s))
                .unwrap()
                .id
        })
        .collect();
    let recipe = RecipeV1 {
        schema_version: 1,
        revision: 1,
        selection: selection.clone(),
        selection_filter: None,
        numbering_order: selection.clone(),
        numbering_scope: Default::default(),
        rules: vec![],
        companions: CompanionPolicy::None,
        destination: Destination::SameDirectory,
        collision_policy: CollisionPolicy::Block,
        metadata_changes: vec![],
    };
    let frozen = FrozenInputs {
        inventory_id: inventory.id,
        generation: inventory.generation,
        now_ns: 0,
        timezone: FrozenTimezone::Utc,
        primary_order: selection,
        numbering_scope: Default::default(),
        values: vec![],
    };
    let mut batch = evaluate(&inventory, &recipe, &frozen).unwrap();
    for row in &mut batch.rows {
        let (_, t) = mapping
            .iter()
            .find(|(s, _)| row.source_path == f.media.join(s))
            .unwrap();
        row.target_path = f.media.join(t);
        row.status = RowStatus::Changed;
    }
    let candidates: Vec<_> = inventory
        .entries
        .iter()
        .map(|e| e.absolute_path.clone())
        .collect();
    let namespace = PlatformFs::default()
        .capture_namespace(&batch.grants, &candidates)
        .unwrap();
    f.plan = plan(
        &inventory,
        &batch,
        &namespace,
        PlanRef {
            session_id: SessionId::new(),
            plan_id: PlanId::new(),
            revision: 1,
        },
    )
    .unwrap();
    assert_eq!(f.plan.counts.blocked, 0, "{:?}", f.plan.issues);
}
pub fn persisted_phase_child(f: &mut Fixture, phase: &str, after: bool) {
    use std::io::Write;
    use vidlib_fileops::{
        events::{CompletedPhase, ContentDigest},
        journal::StepReceipt,
    };
    let original = f.plan.execution.groups[0].steps[0].clone();
    let ExecutionAction::RenameEntry { source, target, .. } = original.action else {
        unreachable!()
    };
    let make = |action| ExecutionStep {
        id: StepId::new(),
        entry_id: original.entry_id,
        group_id: original.group_id,
        action,
    };
    let staging = f.media.join(".vidlib-staging-copy");
    let created = f.media.join("created");
    let stage = make(ExecutionAction::StageCopy {
        source: source.clone(),
        staging: staging.clone(),
    });
    let owner = OwnedEntryPrestate {
        entry: EntryPrestate {
            path: staging.clone(),
            identity: ExpectedIdentity::FromReceipt { step_id: stage.id },
        },
        created_by: stage.id,
    };
    let publish = make(ExecutionAction::PublishCopy {
        staging: owner.clone(),
        target: target.clone(),
        verified_by: stage.id,
    });
    let directory = make(ExecutionAction::CreateDirectory {
        target: created.clone(),
    });
    let changes = vec![MetadataChange::Timestamp {
        field: TimestampField::Modified,
        value_ns: 1_500_000_000_000_000_000,
    }];
    let steps = match phase {
        "stage" => vec![stage.clone()],
        "publish" => vec![stage.clone(), publish.clone()],
        "remove_source" => vec![
            stage.clone(),
            publish.clone(),
            make(ExecutionAction::RemoveSource {
                source: source.clone(),
                published_by: publish.id,
            }),
        ],
        "cleanup" => vec![
            stage.clone(),
            make(ExecutionAction::RemoveOwnedCopy { owned: owner }),
        ],
        "directory" => vec![directory.clone()],
        "directory_cleanup" => vec![
            directory.clone(),
            make(ExecutionAction::RemoveOwnedEmptyDirectory {
                owned: OwnedEntryPrestate {
                    entry: EntryPrestate {
                        path: created.clone(),
                        identity: ExpectedIdentity::FromReceipt {
                            step_id: directory.id,
                        },
                    },
                    created_by: directory.id,
                },
            }),
        ],
        "metadata" => vec![make(ExecutionAction::SetMetadata {
            entry: source.clone(),
            changes,
        })],
        _ => panic!("unknown phase"),
    };
    f.plan.execution.groups[0].steps = steps.clone();
    let _lease = f.coordinator.try_acquire(LeaseKind::Write).unwrap();
    let mut journal = Journal::open(&f.coordinator.database_path()).unwrap();
    let mut native = PlatformFs::default();
    let op = journal.begin(&f.plan.execution).unwrap();
    for (index, step) in steps.iter().enumerate() {
        journal.intent(op, step).unwrap();
        let last = index + 1 == steps.len();
        if last && !after {
            std::process::exit(86);
        }
        let (phase, before_path, after_path, before, post, digest, durability) = match &step.action
        {
            ExecutionAction::StageCopy { .. } => {
                let before = native.inspect(&source.path).unwrap().stamp;
                let mut copy = native.create_staging(&staging).unwrap();
                copy.file.write_all(b"fixture").unwrap();
                let durability = native.refresh_staging(&mut copy).unwrap();
                (
                    CompletedPhase::CopyStaged,
                    Some(encode_path(&source.path)),
                    Some(encode_path(&staging)),
                    Some(before),
                    Some(copy.ownership.stamp),
                    Some(ContentDigest {
                        algorithm: "md5".into(),
                        bytes: md5::compute(b"fixture").0.to_vec(),
                    }),
                    durability,
                )
            }
            ExecutionAction::PublishCopy { .. } => {
                let before = native.inspect(&staging).unwrap().stamp;
                let outcome = native
                    .publish_staging(
                        &OwnedEntry {
                            path: staging.clone(),
                            stamp: before.clone(),
                        },
                        &target,
                    )
                    .unwrap();
                (
                    CompletedPhase::CopyPublished,
                    Some(encode_path(&staging)),
                    Some(encode_path(&target)),
                    Some(before),
                    Some(outcome.observation.stamp),
                    Some(ContentDigest {
                        algorithm: "md5".into(),
                        bytes: md5::compute(b"fixture").0.to_vec(),
                    }),
                    outcome.durability,
                )
            }
            ExecutionAction::CreateDirectory { .. } => {
                let owned = native.create_directory(&created).unwrap();
                (
                    CompletedPhase::DirectoryCreated,
                    None,
                    Some(encode_path(&created)),
                    None,
                    Some(owned.stamp),
                    None,
                    Durability::Synced,
                )
            }
            ExecutionAction::SetMetadata { .. } => {
                let before = native.inspect(&source.path).unwrap().stamp;
                let out = native
                    .set_metadata(
                        &source.path,
                        &before,
                        &MetadataUpdate {
                            modified_ns: Some(1_500_000_000_000_000_000),
                            ..Default::default()
                        },
                    )
                    .unwrap();
                (
                    CompletedPhase::MetadataSet,
                    Some(encode_path(&source.path)),
                    Some(encode_path(&source.path)),
                    Some(before),
                    Some(out.observation.stamp),
                    None,
                    out.durability,
                )
            }
            action => {
                let (path, phase) = match action {
                    ExecutionAction::RemoveSource { .. } => {
                        (source.path.clone(), CompletedPhase::SourceRemoved)
                    }
                    ExecutionAction::RemoveOwnedCopy { .. } => {
                        (staging.clone(), CompletedPhase::OwnedCopyRemoved)
                    }
                    ExecutionAction::RemoveOwnedEmptyDirectory { .. } => {
                        (created.clone(), CompletedPhase::OwnedEmptyDirectoryRemoved)
                    }
                    _ => unreachable!(),
                };
                let before = native.inspect(&path).unwrap().stamp;
                let durability = native
                    .remove_owned(&OwnedEntry {
                        path: path.clone(),
                        stamp: before.clone(),
                    })
                    .unwrap();
                (
                    phase,
                    Some(encode_path(&path)),
                    None,
                    Some(before),
                    None,
                    None,
                    durability,
                )
            }
        };
        if last {
            std::process::exit(86);
        }
        journal
            .complete(
                op,
                &StepReceipt {
                    step_id: step.id,
                    phase,
                    before_path,
                    after_path,
                    before,
                    after: post,
                    digest,
                    observed_ns: 1,
                    durability,
                },
            )
            .unwrap();
    }
    unreachable!()
}
