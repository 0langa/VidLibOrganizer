//! Explicit, selection-bounded ownership. Matching uses original native names;
//! propagation uses the final evaluated primary and never prepares inputs.
use crate::*;
use std::collections::{BTreeMap, BTreeSet};
use vidlib_inventory::{ids::*, EntryKind, EntrySnapshot, Inventory};

pub const COMPANION_EXTENSIONS: &[&str] = &[
    "srt", "ass", "ssa", "vtt", "sup", "idx", "sub", "nfo", "jpg", "jpeg", "png", "webp",
];
const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "wmv", "m4v", "webm", "mpg", "mpeg", "mpe", "m2v", "m2ts", "mts",
    "ts", "vob", "ogv", "flv", "f4v", "3gp", "3g2", "divx", "asf", "rm", "rmvb",
];
const ROLES: &[&str] = &["forced", "sdh", "cc", "poster", "fanart", "thumb"];
#[derive(Clone, Debug)]
pub struct AssociatedAsset {
    pub asset_id: EntryId,
    pub owner: Option<EntryId>,
    /// Exact original spelling, including leading dots and the extension.
    pub retained_suffix: String,
}
#[derive(Clone, Debug)]
pub struct AssociationWarning {
    pub entry_id: EntryId,
    pub asset_id: EntryId,
    pub message: String,
}
#[derive(Clone, Debug)]
pub struct AssociationSet {
    pub selected_ids: Vec<EntryId>,
    pub primary_ids: Vec<EntryId>,
    pub assets: Vec<AssociatedAsset>,
    pub warnings: Vec<AssociationWarning>,
}
fn error(message: &str) -> RenameError {
    RenameError::new(RenameErrorCode::InvalidAssociation, message)
}
fn extension(entry: &EntrySnapshot) -> String {
    entry
        .absolute_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}
fn stem(entry: &EntrySnapshot) -> Option<&str> {
    entry.absolute_path.file_stem()?.to_str()
}
fn recognized(token: &str) -> bool {
    let token = token.to_ascii_lowercase();
    ROLES.contains(&token.as_str())
        || ISO_LANGUAGES
            .split_ascii_whitespace()
            .any(|code| code == token)
}
fn suffix(asset: &EntrySnapshot, primary: &EntrySnapshot) -> Option<String> {
    let rest = stem(asset)?.strip_prefix(stem(primary)?)?;
    if !rest.is_empty() && (!rest.starts_with('.') || !rest[1..].split('.').all(recognized)) {
        return None;
    }
    Some(format!(
        "{rest}.{}",
        asset.absolute_path.extension()?.to_str()?
    ))
}
fn manual_suffix(asset: &EntrySnapshot, primary: &EntrySnapshot) -> Option<String> {
    if let Some(suffix) = suffix(asset, primary) {
        return Some(suffix);
    }
    let stem = stem(asset)?;
    let mut start = stem.len();
    for (offset, _) in stem.match_indices('.').rev() {
        if !recognized(&stem[offset + 1..start]) {
            break;
        }
        start = offset;
    }
    let retained = if recognized(&stem[..start]) {
        format!(".{stem}")
    } else {
        stem[start..].to_owned()
    };
    Some(format!(
        "{retained}.{}",
        asset.absolute_path.extension()?.to_str()?
    ))
}
fn shared(entry: &EntrySnapshot) -> bool {
    matches!(
        stem(entry).unwrap_or("").to_ascii_lowercase().as_str(),
        "poster"
            | "fanart"
            | "thumb"
            | "folder"
            | "cover"
            | "tvshow"
            | "season"
            | "banner"
            | "backdrop"
    )
}
/// Stored IDs are never grants. Manual and automatic owners must both be
/// selected, supported native entries inside the current inventory bindings.
pub fn associate(
    inventory: &Inventory,
    selection: &[EntryId],
    policy: &CompanionPolicy,
) -> Result<AssociationSet, RenameError> {
    let entries = inventory
        .entries
        .iter()
        .map(|e| (e.id, e))
        .collect::<BTreeMap<_, _>>();
    crate::evaluate::validate_inventory(inventory, selection, &entries)?;
    let selected = selection.iter().copied().collect::<BTreeSet<_>>();
    if selected.len() != selection.len() {
        return Err(error("duplicate selected ID"));
    }
    let mut result = AssociationSet {
        selected_ids: selection.to_vec(),
        primary_ids: selection.to_vec(),
        assets: vec![],
        warnings: vec![],
    };
    let manual = match policy {
        CompanionPolicy::None => &[][..],
        CompanionPolicy::Explicit { associations }
        | CompanionPolicy::MatchExact { associations } => associations.as_slice(),
    };
    let mut owners = BTreeMap::new();
    let mut primaries = BTreeSet::new();
    for binding in manual {
        if !entries.contains_key(&binding.primary) {
            return Err(RenameError::new(
                RenameErrorCode::UnknownEntry,
                "unknown companion primary",
            ));
        }
        if !selected.contains(&binding.primary) {
            return Err(RenameError::new(
                RenameErrorCode::UnselectedEntry,
                "unselected companion primary",
            ));
        }
        let primary = entries[&binding.primary];
        if !primaries.insert(binding.primary)
            || primary.kind != EntryKind::File
            || !VIDEO_EXTENSIONS.contains(&extension(primary).as_str())
            || !primary.issues.is_empty()
        {
            return Err(error("owner must be one supported selected video"));
        }
        for id in &binding.companions {
            let asset = entries.get(id).ok_or_else(|| {
                RenameError::new(RenameErrorCode::UnknownEntry, "unknown companion asset")
            })?;
            if !selected.contains(id) {
                return Err(RenameError::new(
                    RenameErrorCode::UnselectedEntry,
                    "unselected companion asset",
                ));
            }
            if owners.insert(*id, binding.primary).is_some()
                || *id == binding.primary
                || asset.kind != EntryKind::File
                || !asset.issues.is_empty()
                || !COMPANION_EXTENSIONS.contains(&extension(asset).as_str())
            {
                return Err(error(
                    "asset must have one owner and a supported companion extension",
                ));
            }
            let retained_suffix = manual_suffix(asset, primary)
                .ok_or_else(|| error("unsupported native companion name"))?;
            result.assets.push(AssociatedAsset {
                asset_id: *id,
                owner: Some(binding.primary),
                retained_suffix,
            });
        }
    }
    let videos = inventory
        .entries
        .iter()
        .filter(|e| {
            e.kind == EntryKind::File
                && VIDEO_EXTENSIONS.contains(&extension(e).as_str())
                && e.issues.is_empty()
        })
        .collect::<Vec<_>>();
    let mut by_stem = BTreeMap::<&str, Vec<&EntrySnapshot>>::new();
    let mut selected_by_folder = BTreeMap::new();
    for primary in &videos {
        if let Some(stem) = stem(primary) {
            by_stem.entry(stem).or_default().push(*primary);
        }
        if selected.contains(&primary.id) {
            selected_by_folder
                .entry(primary.absolute_path.parent())
                .or_insert_with(Vec::new)
                .push(*primary);
        }
    }
    for asset in inventory.entries.iter().filter(|e| {
        e.kind == EntryKind::File && COMPANION_EXTENSIONS.contains(&extension(e).as_str())
    }) {
        if owners.contains_key(&asset.id) {
            continue;
        }
        // Walk only recognized suffix boundaries and index exact stems. This
        // avoids comparing every sidecar with every video in a large inventory.
        let mut candidates = Vec::new();
        if let Some(mut base) = stem(asset) {
            loop {
                if let Some(primaries) = by_stem.get(base) {
                    candidates.extend(
                        primaries
                            .iter()
                            .filter_map(|primary| suffix(asset, primary).map(|s| (*primary, s))),
                    );
                }
                let Some((prefix, tail)) = base.rsplit_once('.') else {
                    break;
                };
                if !recognized(tail) {
                    break;
                }
                base = prefix;
            }
        }
        let local = candidates
            .iter()
            .filter(|(p, _)| p.absolute_path.parent() == asset.absolute_path.parent())
            .collect::<Vec<_>>();
        let is_shared = shared(asset);
        let selected_asset = selected.contains(&asset.id);
        if selected_asset
            && matches!(policy, CompanionPolicy::MatchExact { .. })
            && (is_shared || !candidates.is_empty())
        {
            let owner = if !is_shared
                && local.len() == 1
                && selected.contains(&local[0].0.id)
                && asset.issues.is_empty()
            {
                Some(local[0].0.id)
            } else {
                None
            };
            let retained_suffix = owner
                .and_then(|id| suffix(asset, entries[&id]))
                .unwrap_or_default();
            result.assets.push(AssociatedAsset {
                asset_id: asset.id,
                owner,
                retained_suffix,
            });
            if let Some(owner) = owner {
                owners.insert(asset.id, owner);
            }
            if owner.is_none() {
                result.warnings.push(AssociationWarning { entry_id: asset.id, asset_id: asset.id, message: "Companion kept unchanged: choose one selected owner explicitly (shared artwork, ambiguous owner, or cross-folder asset).".into() });
            }
        }
        // Only selected primaries receive warnings about unselected related assets.
        // Folder-wide artwork is never implicitly granted by this notice.
        let related = if is_shared {
            selected_by_folder
                .get(&asset.absolute_path.parent())
                .cloned()
                .unwrap_or_default()
        } else {
            local
                .iter()
                .map(|(p, _)| *p)
                .filter(|p| selected.contains(&p.id))
                .collect()
        };
        for primary in related {
            if owners.get(&asset.id) != Some(&primary.id) {
                result.warnings.push(AssociationWarning { entry_id: primary.id, asset_id: asset.id, message: format!("Related asset {} is not bound to this primary; it will not inherit its name.", asset.display_name) });
            }
        }
    }
    // Selected VobSub peers must either remain independent together or share
    // one owner. Manual-only mode never silently adds the other half.
    let mut vobsub = BTreeMap::<_, Vec<_>>::new();
    for id in selection {
        let entry = entries[id];
        if matches!(extension(entry).as_str(), "idx" | "sub") {
            vobsub
                .entry((entry.absolute_path.parent(), stem(entry)))
                .or_default()
                .push(entry);
        }
    }
    for pair in vobsub.values() {
        for entry in pair {
            if pair.iter().any(|peer| {
                extension(peer) != extension(entry) && owners.get(&entry.id) != owners.get(&peer.id)
            }) {
                return Err(error(
                    "selected idx/sub pair requires the same explicit owner",
                ));
            }
        }
    }
    let assets = result
        .assets
        .iter()
        .map(|a| a.asset_id)
        .collect::<BTreeSet<_>>();
    result.primary_ids.retain(|id| !assets.contains(id));
    Ok(result)
}
/// Resolve ownership before freezing order or requesting any hashes/metadata.
/// Preserve surviving user order; append newly independent selected primaries.
pub fn prepare_associations(
    inventory: &Inventory,
    recipe: &mut RecipeV1,
) -> Result<AssociationSet, RenameError> {
    let associations = associate(inventory, &recipe.selection, &recipe.companions)?;
    let primaries = associations
        .primary_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut updated = recipe.clone();
    let mut seen = BTreeSet::new();
    updated
        .numbering_order
        .retain(|id| primaries.contains(id) && seen.insert(*id));
    updated.numbering_order.extend(
        associations
            .primary_ids
            .iter()
            .filter(|id| seen.insert(**id))
            .copied(),
    );
    updated.validate()?;
    *recipe = updated;
    Ok(associations)
}
/// Reapply after primary literal overrides too. Companion paths remain in their
/// selected original folders; a binding grants names, not a destination move.
pub fn propagate_names(
    evaluated: &mut EvaluatedBatch,
    associations: &AssociationSet,
) -> Result<(), RenameError> {
    let originals = evaluated
        .rows
        .iter()
        .map(|r| (r.entry_id, r.clone()))
        .collect::<BTreeMap<_, _>>();
    for asset in &associations.assets {
        let Some(owner) = asset.owner else {
            continue;
        };
        let primary = originals
            .get(&owner)
            .ok_or_else(|| error("missing evaluated primary"))?;
        let row = evaluated
            .rows
            .iter_mut()
            .find(|r| r.entry_id == asset.asset_id)
            .ok_or_else(|| error("missing selected companion row"))?;
        row.primary_id = owner;
        row.group_id = primary.group_id;
        if row
            .issues
            .iter()
            .any(|i| i.code == RenameErrorCode::UnsupportedName)
        {
            continue;
        }
        row.issues
            .retain(|issue| issue.code != RenameErrorCode::InvalidAssociation);
        if matches!(primary.status, RowStatus::Blocked | RowStatus::NeedsReview) {
            row.status = RowStatus::Blocked;
            row.issues.push(PlanIssue {
                code: RenameErrorCode::InvalidAssociation,
                entry_id: Some(row.entry_id),
                rule_id: None,
                message: "Companion primary needs resolution before propagation".into(),
            });
            continue;
        }
        let stem = primary
            .target_path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| error("unsupported evaluated primary name"))?;
        let name = format!("{stem}{}", asset.retained_suffix);
        row.target_path = row.source_path.with_file_name(&name);
        row.new_display_name = name.clone();
        row.new_display_path = row.target_path.to_string_lossy().into_owned();
        row.status = if row.target_path == row.source_path {
            RowStatus::Unchanged
        } else {
            RowStatus::Changed
        };
        row.explanations.retain(|e| e.rule_id != "companion");
        row.explanations.push(RuleExplanation {
            rule_id: "companion".into(),
            before: row.old_display_name.clone(),
            after: name,
        });
    }
    Ok(())
}

// ISO 639-1 / 639-2 language codes, including bibliographic aliases.
const ISO_LANGUAGES: &str = "aa aar ab abk ace ach ada ady ae af afa afh afr ain ak aka akk alb ale alg alt am amh an ang anp apa ar ara arc arg arm arn arp art arw as asm ast ath aus av ava ave awa ay aym az aze ba bad bai bak bal bam ban baq bas bat be bej bel bem ben ber bg bh bho bi bih bik bin bis bla bm bn bnt bo bod bos br bra bre bs btk bua bug bul bur byn ca cad cai car cat cau ce ceb cel ces ch cha chb che chg chi chk chm chn cho chp chr chu chv chy cmc cnr co cop cor cos cpe cpf cpp cr cre crh crp cs csb cu cus cv cy cym cze da dak dan dar day de del den deu dgr din div doi dra dsb dua dum dut dv dyu dz dzo ee efi egy eka el ell elx en eng enm eo epo es est et eu eus ewe ewo fa fan fao fas fat ff fi fij fil fin fiu fj fo fon fr fra fre frm fro frr frs fry ful fur fy ga gaa gay gba gd gem geo ger gez gil gl gla gle glg glv gmh gn goh gon gor got grb grc gre grn gsw gu guj gv gwi ha hai hat hau haw he heb her hi hil him hin hit hmn hmo ho hr hrv hsb ht hu hun hup hy hye hz ia iba ibo ice id ido ie ig ii iii ijo ik iku ile ilo ina inc ind ine inh io ipk ira iro is isl it ita iu ja jav jbo jpn jpr jrb jv ka kaa kab kac kal kam kan kar kas kat kau kaw kaz kbd kg kha khi khm kho ki kik kin kir kj kk kl km kmb kn ko kok kom kon kor kos kpe kr krc krl kro kru ks ku kua kum kur kut kv kw ky la lad lah lam lao lat lav lb lez lg li lim lin lit ln lo lol loz lt ltz lu lua lub lug lui lun luo lus lv mac mad mag mah mai mak mal man mao map mar mas may mdf mdr men mg mga mh mi mic min mis mk mkd mkh ml mlg mlt mn mnc mni mno moh mon mos mr mri ms msa mt mul mun mus mwl mwr my mya myn myv na nah nai nap nau nav nb nbl nd nde ndo nds ne nep new ng nia nic niu nl nld nn nno no nob nog non nor nqo nr nso nub nv nwc ny nya nym nyn nyo nzi oc oci oj oji om or ori orm os osa oss ota oto pa paa pag pal pam pan pap pau peo per phi phn pi pl pli pol pon por pra pro ps pt pus qu que raj rap rar rm rn ro roa roh rom ron ru rum run rup rus rw sa sad sag sah sai sal sam san sas sat sc scn sco sd se sel sem sg sga sgn shn si sid sin sio sit sk sl sla slk slo slv sm sma sme smi smj smn smo sms sn sna snd snk so sog som son sot spa sq sqi sr srd srn srp srr ss ssa ssw st su suk sun sus sux sv sw swa swe syc syr ta tah tai tam tat te tel tem ter tet tg tgk tgl th tha ti tib tig tir tiv tk tkl tl tlh tli tmh tn to tog ton tpi tr ts tsi tsn tso tt tuk tum tup tur tut tvl tw twi ty tyv udm ug uga uig uk ukr umb und ur urd uz uzb vai ve ven vi vie vo vol vot wa wak wal war was wel wen wln wo wol xal xh xho yao yap yi yid yo yor ypk za zap zbl zen zgh zh zha zho znd zu zul zun zxx zza";
