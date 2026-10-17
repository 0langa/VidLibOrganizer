//! Desktop state owns paths and process-scoped registries, never SQLite handles.
use crate::jobs::Jobs;
use std::{
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Arc, Mutex},
};
use vidlib_fileops::{coordinator::Coordinator, OpError};
use vidlib_inventory::ids::SessionId;
use vidlib_rename::PlanStore;

#[derive(Clone)]
pub struct AppState {
    pub rename: Arc<Mutex<crate::commands::rename::RenameState>>,
    pub(crate) jobs: Arc<Mutex<Jobs>>,
    pub plans: Arc<Mutex<PlanRegistry>>,
    pub coordinator: Arc<Coordinator>,
    pub operations: PathBuf,
    pub blueprint: PathBuf,
    pub(crate) preferences: PathBuf,
    pub(crate) preferences_lock: Arc<Mutex<()>>,
    pub(crate) rename_preferences_lock: Arc<Mutex<()>>,
}
impl AppState {
    pub fn open(app_data: &Path, preferences: PathBuf) -> Result<Self, OpError> {
        let coordinator = Arc::new(Coordinator::open(app_data)?);
        Ok(Self {
            rename: Arc::new(Mutex::new(crate::commands::rename::RenameState::new(
                vidlib_inventory::ProtectedPolicy {
                    protected_roots: vec![coordinator
                        .database_path()
                        .parent()
                        .expect("absolute app data database")
                        .to_path_buf()],
                    ..Default::default()
                },
            ))),
            jobs: Arc::new(Mutex::new(Jobs::new(coordinator.clone()))),
            plans: Arc::new(Mutex::new(PlanRegistry::new(coordinator.clone()))),
            operations: coordinator.database_path(),
            blueprint: coordinator.blueprint_path(),
            coordinator,
            preferences,
            preferences_lock: Arc::new(Mutex::new(())),
            rename_preferences_lock: Arc::new(Mutex::new(())),
        })
    }
    /// True prevents closing while a local writer/recovery lease is active.
    pub fn request_close(&self) -> bool {
        let Ok(jobs) = self.jobs.lock() else {
            return true;
        };
        if jobs.acting || self.coordinator.has_active_writer() {
            return true;
        }
        jobs.cancel.store(true, Ordering::Relaxed);
        crate::commands::rename::cancel_all(self);
        false
    }
}

/// The only desktop access to plan authority. Paths and confirmations stay backend-owned.
pub struct PlanRegistry {
    coordinator: Arc<Coordinator>,
    session: SessionId,
    store: PlanStore,
}
impl PlanRegistry {
    pub fn new(coordinator: Arc<Coordinator>) -> Self {
        let session = SessionId::new();
        Self {
            coordinator,
            session,
            store: PlanStore::new(session),
        }
    }
    pub fn session_id(&self) -> SessionId {
        self.session
    }
    fn check(
        &mut self,
        lease: &vidlib_fileops::coordinator::Lease,
        epochs: vidlib_fileops::coordinator::Epochs,
    ) -> Result<(), OpError> {
        use vidlib_fileops::{coordinator::LeaseKind, ErrorCode, Phase};
        if !self.coordinator.owns_lease(lease) || lease.kind() == LeaseKind::Recovery {
            return Err(OpError::new(
                ErrorCode::PermissionDenied,
                Phase::Verify,
                "plan authority requires a lease for this collection",
            ));
        }
        if let Err(error) = lease.verify_epochs(epochs) {
            self.store.invalidate();
            return Err(error);
        }
        Ok(())
    }
    pub fn publish(
        &mut self,
        plan: vidlib_rename::RenamePlan,
        lease: &vidlib_fileops::coordinator::Lease,
    ) -> Result<(), OpError> {
        self.check(lease, plan_epochs(&plan))?;
        self.store.publish(plan).map_err(plan_error)
    }
    pub fn current(
        &mut self,
        version: vidlib_rename::PlanRef,
        lease: &vidlib_fileops::coordinator::Lease,
    ) -> Result<&vidlib_rename::RenamePlan, OpError> {
        let epochs = plan_epochs(self.store.current(version).map_err(plan_error)?);
        self.check(lease, epochs)?;
        self.store.current(version).map_err(plan_error)
    }
    pub fn snapshot(
        &mut self,
        version: vidlib_rename::PlanRef,
        lease: &vidlib_fileops::coordinator::Lease,
    ) -> Result<Arc<vidlib_rename::RenamePlan>, OpError> {
        self.current(version, lease)?;
        self.store.snapshot(version).map_err(plan_error)
    }
    pub fn consume_owned(
        &mut self,
        version: vidlib_rename::PlanRef,
        nonce: &str,
        lease: &vidlib_fileops::coordinator::Lease,
    ) -> Result<Arc<vidlib_rename::RenamePlan>, OpError> {
        self.consume_confirmation(version, nonce, lease)?;
        self.store.snapshot(version).map_err(plan_error)
    }
    pub fn confirm(
        &mut self,
        version: vidlib_rename::PlanRef,
        nonce: String,
        lease: &vidlib_fileops::coordinator::Lease,
    ) -> Result<(), OpError> {
        let epochs = plan_epochs(self.store.current(version).map_err(plan_error)?);
        self.check(lease, epochs)?;
        self.store.confirm(version, nonce).map_err(plan_error)
    }
    pub fn consume_confirmation(
        &mut self,
        version: vidlib_rename::PlanRef,
        nonce: &str,
        lease: &vidlib_fileops::coordinator::Lease,
    ) -> Result<&vidlib_rename::RenamePlan, OpError> {
        if lease.kind() != vidlib_fileops::coordinator::LeaseKind::Write {
            return Err(OpError::new(
                vidlib_fileops::ErrorCode::PermissionDenied,
                vidlib_fileops::Phase::Verify,
                "confirmation consumption requires a write lease",
            ));
        }
        let epochs = plan_epochs(self.store.current(version).map_err(plan_error)?);
        self.check(lease, epochs)?;
        self.store
            .consume_confirmation(version, nonce)
            .map_err(plan_error)
    }
    pub fn invalidate(&mut self) {
        self.store.invalidate();
    }
}
fn plan_epochs(plan: &vidlib_rename::RenamePlan) -> vidlib_fileops::coordinator::Epochs {
    let provenance = &plan.execution.provenance;
    vidlib_fileops::coordinator::Epochs {
        namespace: provenance.namespace_epoch,
        policy: provenance.policy_epoch,
    }
}
fn plan_error(error: vidlib_rename::RenameError) -> OpError {
    vidlib_fileops::OpError::new(
        vidlib_fileops::ErrorCode::StaleState,
        vidlib_fileops::Phase::Verify,
        error.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use vidlib_fileops::{coordinator::LeaseKind, ErrorCode};
    use vidlib_inventory::{ids::*, *};
    use vidlib_rename::*;
    fn plan(session: SessionId) -> RenamePlan {
        let inventory = Inventory {
            id: InventoryId::new(),
            generation: 1,
            roots: vec![],
            entries: vec![],
            coverage: vec![],
            window: ObservationWindow::default(),
            issues: vec![],
        };
        let inputs = FrozenInputs {
            inventory_id: inventory.id,
            generation: 1,
            now_ns: 0,
            timezone: FrozenTimezone::Utc,
            primary_order: vec![],
            numbering_scope: Default::default(),
            values: vec![],
        };
        let recipe = RecipeV1 {
            schema_version: 1,
            revision: 1,
            selection: vec![],
            selection_filter: None,
            numbering_order: vec![],
            numbering_scope: Default::default(),
            rules: vec![],
            companions: CompanionPolicy::None,
            destination: Destination::SameDirectory,
            collision_policy: CollisionPolicy::Block,
            metadata_changes: vec![],
        };
        let version = PlanRef {
            session_id: session,
            plan_id: PlanId::new(),
            revision: 1,
        };
        RenamePlan {
            inverse_of: None,
            version,
            inventory_id: inventory.id,
            generation: 1,
            recipe_revision: 1,
            counts: PlanCounts::from_rows(&[]).unwrap(),
            rows: vec![],
            issues: vec![],
            frozen_inputs: inputs.clone(),
            grants: GrantSet::default(),
            execution: ExecutionPlan {
                inverse_of: None,
                expected_namespace: Default::default(),
                groups: vec![],
                provenance: InspectionProvenance {
                    approved_plan: version,
                    inventory,
                    recipe,
                    frozen_inputs: inputs,
                    grants: GrantSet::default(),
                    namespace_epoch: 0,
                    policy_epoch: 0,
                },
            },
        }
    }

    // A published, confirmed plan must lose authority when another instance changes policy.
    #[test]
    fn foreign_epoch_revokes_plan_queries() {
        let dir = tempfile::tempdir().unwrap();
        let coordinator = Arc::new(Coordinator::open(dir.path()).unwrap());
        let mut plans = PlanRegistry::new(coordinator.clone());
        let plan = plan(plans.session_id());
        let version = plan.version;
        {
            let lease = coordinator.try_acquire(LeaseKind::Read).unwrap();
            plans.publish(plan, &lease).unwrap();
        }
        Coordinator::open(dir.path())
            .unwrap()
            .try_acquire(LeaseKind::Write)
            .unwrap()
            .advance_namespace()
            .unwrap();
        let lease = coordinator.try_acquire(LeaseKind::Read).unwrap();
        assert_eq!(
            plans.current(version, &lease).unwrap_err().code,
            ErrorCode::StaleState
        );
    }

    #[test]
    fn valid_confirmation_is_consumed_only_under_its_collection_writer() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let coordinator = Arc::new(Coordinator::open(dir.path()).unwrap());
        let mut plans = PlanRegistry::new(coordinator.clone());
        let plan = plan(plans.session_id());
        let version = plan.version;
        {
            let lease = coordinator.try_acquire(LeaseKind::Read).unwrap();
            plans.publish(plan, &lease).unwrap();
            plans.confirm(version, "nonce".into(), &lease).unwrap();
        }
        let wrong = Coordinator::open(other.path())
            .unwrap()
            .try_acquire(LeaseKind::Write)
            .unwrap();
        assert_eq!(
            plans
                .consume_confirmation(version, "nonce", &wrong)
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
        let writer = coordinator.try_acquire(LeaseKind::Write).unwrap();
        assert_eq!(
            plans
                .consume_confirmation(version, "nonce", &writer)
                .unwrap()
                .version,
            version
        );
        assert!(plans
            .consume_confirmation(version, "nonce", &writer)
            .is_err());
    }

    #[test]
    fn foreign_epoch_revokes_plan_and_nonce() {
        let dir = tempfile::tempdir().unwrap();
        let coordinator = Arc::new(Coordinator::open(dir.path()).unwrap());
        let mut plans = PlanRegistry::new(coordinator.clone());
        let plan = plan(plans.session_id());
        let version = plan.version;
        {
            let lease = coordinator.try_acquire(LeaseKind::Read).unwrap();
            plans.publish(plan, &lease).unwrap();
            plans
                .confirm(version, "backend-nonce".into(), &lease)
                .unwrap();
        }
        Coordinator::open(dir.path())
            .unwrap()
            .try_acquire(LeaseKind::Write)
            .unwrap()
            .advance_policy()
            .unwrap();
        let lease = coordinator.try_acquire(LeaseKind::Write).unwrap();
        assert_eq!(
            plans
                .consume_confirmation(version, "backend-nonce", &lease)
                .unwrap_err()
                .code,
            ErrorCode::StaleState
        );
        assert!(
            plans.store.current(version).is_err(),
            "stale plan must be removed from registry"
        );
    }

    #[test]
    fn read_lease_cannot_consume_confirmation() {
        let dir = tempfile::tempdir().unwrap();
        let coordinator = Arc::new(Coordinator::open(dir.path()).unwrap());
        let mut plans = PlanRegistry::new(coordinator.clone());
        let plan = plan(plans.session_id());
        let version = plan.version;
        let lease = coordinator.try_acquire(LeaseKind::Read).unwrap();
        plans.publish(plan, &lease).unwrap();
        plans
            .confirm(version, "backend-nonce".into(), &lease)
            .unwrap();
        assert_eq!(
            plans
                .consume_confirmation(version, "backend-nonce", &lease)
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
    }

    #[test]
    fn close_waits_for_recovery_and_cancels_scans() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::open(dir.path(), dir.path().join("libraries.json")).unwrap();
        let recovery = state.coordinator.try_acquire(LeaseKind::Recovery).unwrap();
        assert!(state.request_close());
        drop(recovery);
        let (_id, cancel, _lease) = state.jobs.lock().unwrap().begin().unwrap();
        assert!(!state.request_close());
        assert!(cancel.load(Ordering::Relaxed));
    }
}
