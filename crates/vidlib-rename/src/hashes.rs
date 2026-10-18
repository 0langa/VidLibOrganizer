//! Lazy input preparation and inventory-scoped generated values.
use crate::*;
use sha2::Digest;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Read},
};
use vidlib_inventory::{ids::EntryId, Inventory, JobControl};
#[derive(Clone, Debug)]
pub struct RequestedInput {
    pub scope: FrozenScope,
    pub entry_id: EntryId,
    pub rule_id: Option<String>,
    pub key: FieldKey,
}
#[derive(Clone, Debug)]
pub struct InputRequirements {
    pub primary_order: Vec<EntryId>,
    pub numbering_scope: NumberingScope,
    pub fields: Vec<RequestedInput>,
    pub timezone: FrozenTimezone,
}
impl InputRequirements {
    pub fn from_recipe(recipe: &RecipeV1, timezone: FrozenTimezone) -> Result<Self, RenameError> {
        recipe.validate()?;
        let bytes = serde_json::to_vec(recipe)
            .map_err(|e| error(RenameErrorCode::InvalidRecipe, e.to_string()))?;
        if bytes.len() > 1024 * 1024 {
            return Err(error(
                RenameErrorCode::RecipeTooLarge,
                "recipe exceeds 1 MiB",
            ));
        }
        let mut fields = vec![];
        fn collect(parts: &[TemplatePart], keys: &mut Vec<FieldKey>) {
            for part in parts {
                match part {
                    TemplatePart::Field { key, .. } => keys.push(key.clone()),
                    TemplatePart::Optional(parts) => collect(parts, keys),
                    _ => {}
                }
            }
        }
        for rule in recipe.rules.iter().filter(|r| r.enabled) {
            let mut keys = vec![];
            match &rule.parameters {
                RuleParameters::Template { parts } => collect(parts, &mut keys),
                RuleParameters::Date {
                    operation: DateOperation::Insert { source, .. },
                } => keys.push(FieldKey::Date { source: *source }),
                _ => {}
            }
            for entry_id in &recipe.numbering_order {
                for key in &keys {
                    fields.push(RequestedInput {
                        scope: Default::default(),
                        entry_id: *entry_id,
                        rule_id: Some(rule.id.clone()),
                        key: key.clone(),
                    });
                }
            }
        }
        if let Destination::CopyTo {
            layout: DestinationLayout::FolderTemplate { parts },
            ..
        }
        | Destination::MoveTo {
            layout: DestinationLayout::FolderTemplate { parts },
            ..
        } = &recipe.destination
        {
            let mut keys = vec![];
            collect(parts, &mut keys);
            for entry_id in &recipe.numbering_order {
                for key in &keys {
                    fields.push(RequestedInput {
                        scope: FrozenScope::Destination,
                        entry_id: *entry_id,
                        rule_id: None,
                        key: key.clone(),
                    });
                }
            }
        }
        Ok(Self {
            primary_order: recipe.numbering_order.clone(),
            numbering_scope: recipe.numbering_scope,
            fields,
            timezone,
        })
    }
}
#[derive(Clone, Default)]
pub struct FrozenInputCache {
    pub(crate) retained: Option<FrozenInputs>,
    pub(crate) metadata_cache: Vec<crate::metadata::CacheEntry>,
    // Only hashes computed through verified native content enter this cache.
    // Frozen user values/missing decisions are scoped owners, not shared proof.
    content_hashes: BTreeMap<(EntryId, HashAlgorithm), String>,
    /// Cumulative bytes read by this cache, including cancelled preparations.
    pub content_bytes_read: u64,
}
impl FrozenInputCache {
    /// Seed explicit metadata and user-frozen values; validated again by evaluation.
    pub fn from_inputs(inputs: FrozenInputs) -> Self {
        Self {
            retained: Some(inputs),
            content_bytes_read: 0,
            metadata_cache: vec![],
            content_hashes: BTreeMap::new(),
        }
    }
    /// Explicit regeneration; inventory ID/generation changes also reset retention.
    pub fn regenerate(&mut self) {
        self.retained = None;
        self.metadata_cache.clear();
        self.content_hashes.clear();
    }
    pub fn prepare(
        &mut self,
        request: &InputRequirements,
        inventory: &Inventory,
        control: &JobControl,
    ) -> Result<FrozenInputs, RenameError> {
        check_cancel(control)?;
        if !crate::dates::valid_timezone(&request.timezone) {
            return Err(error(
                RenameErrorCode::InvalidFrozenInputs,
                "invalid requested timezone",
            ));
        }
        let entries = inventory
            .entries
            .iter()
            .map(|e| (e.id, e))
            .collect::<BTreeMap<_, _>>();
        let selected = request
            .primary_order
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if entries.len() != inventory.entries.len()
            || selected.len() != request.primary_order.len()
            || selected.iter().any(|id| !entries.contains_key(id))
            || request
                .fields
                .iter()
                .any(|field| !selected.contains(&field.entry_id))
        {
            return Err(error(
                RenameErrorCode::InvalidNumberingOrder,
                "requested fields must belong to distinct observed primaries",
            ));
        }
        let same_inventory = self.retained.as_ref().is_some_and(|old| {
            old.inventory_id == inventory.id && old.generation == inventory.generation
        });
        let mut content_hashes = if same_inventory {
            self.content_hashes.clone()
        } else {
            BTreeMap::new()
        };
        let mut retained = self
            .retained
            .clone()
            .filter(|old| {
                old.inventory_id == inventory.id && old.generation == inventory.generation
            })
            .unwrap_or_else(|| FrozenInputs {
                inventory_id: inventory.id,
                generation: inventory.generation,
                now_ns: now_ns(),
                timezone: request.timezone.clone(),
                primary_order: vec![],
                numbering_scope: Default::default(),
                values: vec![],
            });
        retained.timezone = request.timezone.clone();
        retained.primary_order = request.primary_order.clone();
        retained.numbering_scope = request.numbering_scope;
        let counter_order = crate::selection::scoped_order(
            inventory,
            &request.primary_order,
            request.numbering_scope,
        )?;
        let counter_ordinals = counter_order
            .iter()
            .enumerate()
            .map(|(ordinal, id)| (*id, ordinal as i128 + 1))
            .collect::<BTreeMap<_, _>>();
        // Local indexes preserve first-value lookup and serialized vector order.
        let mut retained_index = BTreeMap::new();
        for (index, value) in retained.values.iter().enumerate() {
            retained_index.entry(value_key(value)).or_insert(index);
        }
        let mut output_keys = BTreeSet::new();
        // Hashes are shared per entry, independent of how many rules request them.
        let mut requested_hashes = BTreeMap::<EntryId, BTreeSet<HashAlgorithm>>::new();
        for field in &request.fields {
            if let FieldKey::Hash { algorithm } = field.key {
                // Explicit choices still require the original native binding
                // check, although they need no computed value or content read.
                let algorithms = requested_hashes.entry(field.entry_id).or_default();
                if lookup(&retained, &retained_index, field).is_none() {
                    algorithms.insert(algorithm);
                }
            }
        }
        for (id, algorithms) in requested_hashes {
            check_cancel(control)?;
            if entries[&id].kind != vidlib_inventory::EntryKind::File {
                continue;
            }
            let needed = algorithms
                .into_iter()
                .filter(|algorithm| !content_hashes.contains_key(&(id, *algorithm)))
                .collect::<BTreeSet<_>>();
            // Even cached hashes revalidate the source binding/material stamp, without reading contents.
            let hashes = vidlib_inventory::with_verified_content(inventory, id, |reader| {
                hash_reader(reader, &needed, control, &mut self.content_bytes_read)
            })
            .map_err(|e| {
                if control.is_cancelled() {
                    error(RenameErrorCode::Cancelled, "input preparation cancelled")
                } else {
                    error(RenameErrorCode::StaleInputs, e.to_string())
                }
            })?;
            for (algorithm, hash) in hashes {
                content_hashes.insert((id, algorithm), hash);
            }
        }
        let mut output = retained.clone();
        output.values.clear();
        for field in &request.fields {
            check_cancel(control)?;
            if !needs_frozen(&field.key) {
                continue;
            }
            let previous = if field.key == FieldKey::Counter
                && !counter_ordinals.contains_key(&field.entry_id)
            {
                Some(FrozenValue {
                    scope: field.scope,
                    entry_id: field.entry_id,
                    rule_id: field.rule_id.clone(),
                    key: field.key.clone(),
                    value: FrozenFieldValue::Missing {
                        decision: MissingDecision::Block,
                    },
                })
            } else {
                lookup(&retained, &retained_index, field)
                    .map(|previous| {
                        let mut value = previous.clone();
                        if matches!(field.key, FieldKey::Hash { .. }) {
                            value.scope = field.scope;
                            value.rule_id = field.rule_id.clone();
                        }
                        value
                    })
                    .or_else(|| {
                        let FieldKey::Hash { algorithm } = field.key else {
                            return None;
                        };
                        content_hashes
                            .get(&(field.entry_id, algorithm))
                            .map(|hash| FrozenValue {
                                scope: field.scope,
                                entry_id: field.entry_id,
                                rule_id: field.rule_id.clone(),
                                key: field.key.clone(),
                                value: FrozenFieldValue::Text(hash.clone()),
                            })
                    })
            };
            let value = previous.unwrap_or_else(|| FrozenValue {
                scope: field.scope,
                entry_id: field.entry_id,
                rule_id: field.rule_id.clone(),
                key: field.key.clone(),
                value: match field.key {
                    FieldKey::Uuid => FrozenFieldValue::Text(uuid::Uuid::new_v4().to_string()),
                    FieldKey::Random => {
                        FrozenFieldValue::Integer(i128::from(rand::random::<u64>()))
                    }
                    FieldKey::Counter => counter_ordinals
                        .get(&field.entry_id)
                        .map(|ordinal| FrozenFieldValue::Integer(*ordinal))
                        .unwrap_or(FrozenFieldValue::Missing {
                            decision: MissingDecision::Block,
                        }),
                    _ => FrozenFieldValue::Missing {
                        decision: MissingDecision::Block,
                    },
                },
            });
            // Default counters follow the current explicit primary vector. Explicit overrides stay frozen.
            if !matches!(field.key, FieldKey::Counter | FieldKey::Hash { .. })
                && lookup(&retained, &retained_index, field).is_none()
            {
                retained_index.insert(value_key(&value), retained.values.len());
                retained.values.push(value.clone());
            }
            if output_keys.insert(value_key(&value)) {
                output.values.push(value);
            }
        }
        check_cancel(control)?;
        self.retained = Some(retained);
        self.content_hashes = content_hashes;
        Ok(output)
    }
}
type ValueKey = (FrozenScope, EntryId, Option<String>, FieldKey);
fn value_key(value: &FrozenValue) -> ValueKey {
    (
        value.scope,
        value.entry_id,
        value.rule_id.clone(),
        value.key.clone(),
    )
}
fn lookup<'a>(
    inputs: &'a FrozenInputs,
    index: &BTreeMap<ValueKey, usize>,
    field: &RequestedInput,
) -> Option<&'a FrozenValue> {
    let key = (
        field.scope,
        field.entry_id,
        field.rule_id.clone(),
        field.key.clone(),
    );
    index
        .get(&key)
        .or_else(|| index.get(&(field.scope, field.entry_id, None, field.key.clone())))
        .map(|index| &inputs.values[*index])
}
fn needs_frozen(key: &FieldKey) -> bool {
    matches!(
        key,
        FieldKey::Uuid
            | FieldKey::Random
            | FieldKey::Counter
            | FieldKey::Hash { .. }
            | FieldKey::Media { .. }
            | FieldKey::WindowsProperty { .. }
            | FieldKey::Date {
                source: DateSource::Recorded
            }
    )
}
fn now_ns() -> i128 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(value) => value.as_nanos() as i128,
        Err(error) => -(error.duration().as_nanos() as i128),
    }
}
fn error(code: RenameErrorCode, message: impl Into<String>) -> RenameError {
    RenameError::new(code, message)
}
fn check_cancel(control: &JobControl) -> Result<(), RenameError> {
    if control.is_cancelled() {
        Err(error(
            RenameErrorCode::Cancelled,
            "input preparation cancelled",
        ))
    } else {
        Ok(())
    }
}
pub fn freeze_inputs(
    request: &InputRequirements,
    inventory: &Inventory,
    control: &JobControl,
) -> Result<FrozenInputs, RenameError> {
    FrozenInputCache::default().prepare(request, inventory, control)
}
enum Hasher {
    Crc32(crc32fast::Hasher),
    Md5(md5::Md5),
    Sha1(sha1::Sha1),
    Sha256(sha2::Sha256),
    Sha512(sha2::Sha512),
    Blake3(Box<blake3::Hasher>),
}
impl Hasher {
    fn new(algorithm: HashAlgorithm) -> Self {
        match algorithm {
            HashAlgorithm::Crc32 => Self::Crc32(crc32fast::Hasher::new()),
            HashAlgorithm::Md5 => Self::Md5(md5::Md5::new()),
            HashAlgorithm::Sha1 => Self::Sha1(sha1::Sha1::new()),
            HashAlgorithm::Sha256 => Self::Sha256(sha2::Sha256::new()),
            HashAlgorithm::Sha512 => Self::Sha512(sha2::Sha512::new()),
            HashAlgorithm::Blake3 => Self::Blake3(Box::new(blake3::Hasher::new())),
        }
    }
    fn update(&mut self, bytes: &[u8]) {
        match self {
            Self::Crc32(h) => h.update(bytes),
            Self::Md5(h) => h.update(bytes),
            Self::Sha1(h) => h.update(bytes),
            Self::Sha256(h) => h.update(bytes),
            Self::Sha512(h) => h.update(bytes),
            Self::Blake3(h) => {
                h.update(bytes);
            }
        }
    }
    fn finish(self) -> String {
        match self {
            Self::Crc32(h) => format!("{:08x}", h.finalize()),
            Self::Md5(h) => format!("{:x}", h.finalize()),
            Self::Sha1(h) => format!("{:x}", h.finalize()),
            Self::Sha256(h) => format!("{:x}", h.finalize()),
            Self::Sha512(h) => format!("{:x}", h.finalize()),
            Self::Blake3(h) => h.finalize().to_hex().to_string(),
        }
    }
}
fn hash_reader(
    reader: &mut dyn Read,
    algorithms: &BTreeSet<HashAlgorithm>,
    control: &JobControl,
    bytes_read: &mut u64,
) -> io::Result<BTreeMap<HashAlgorithm, String>> {
    if algorithms.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut hashers = algorithms
        .iter()
        .map(|a| (*a, Hasher::new(*a)))
        .collect::<Vec<_>>();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        if control.is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "hash cancelled"));
        }
        let length = match reader.read(&mut buffer) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            value => value?,
        };
        if length == 0 {
            break;
        }
        *bytes_read = bytes_read.saturating_add(length as u64);
        for (_, hasher) in &mut hashers {
            hasher.update(&buffer[..length]);
        }
    }
    if control.is_cancelled() {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "hash cancelled"));
    }
    Ok(hashers.into_iter().map(|(a, h)| (a, h.finish())).collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hash_cancellation_is_checked_between_bounded_chunks() {
        struct CancellingReader {
            control: JobControl,
            calls: usize,
        }
        impl Read for CancellingReader {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                assert!(bytes.len() <= 1024 * 1024);
                self.calls += 1;
                assert_eq!(self.calls, 1, "must stop before another read");
                bytes.fill(b'a');
                self.control.cancel();
                Ok(bytes.len())
            }
        }
        let control = JobControl::default();
        let mut reader = CancellingReader {
            control: control.clone(),
            calls: 0,
        };
        let mut count = 0;
        let result = hash_reader(
            &mut reader,
            &BTreeSet::from([HashAlgorithm::Sha256]),
            &control,
            &mut count,
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert_eq!(count, 1024 * 1024);
    }
}
