use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use super::{
    JournalPhase, PublicationError, PublicationJournal, PublicationRole, PublicationSet,
    RecoveryDirection, TuneOutputSet, TunePublishArtifacts,
    journal::{
        ContentIdentity, decode_publication_journal, encode_publication_journal, identity_at,
    },
    lock::{acquire_ids, hex},
    platform::{
        AdvisoryLock, atomic_no_replace, atomic_replace, create_private, make_executable,
        open_private_nofollow, random_transaction_id, read_private_nofollow, sync_directory,
    },
};

const MAX_JOURNAL_BYTES: u64 = 128 * 1024;

/// Deterministic crash points used by the exhaustive recovery harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationFault {
    AfterStages,
    AfterJournalPrivate(JournalPhase),
    AfterJournalUpdate(JournalPhase),
    AfterPhase(JournalPhase),
    AfterBackups,
    AfterDecision,
    AfterSidecars,
    AfterPrimary,
}

#[derive(Debug, Clone)]
struct JournalFiles {
    active: Option<(PathBuf, PublicationJournal)>,
    update: Option<(PathBuf, PublicationJournal)>,
}

pub(crate) fn acquire_stable_closure_and_recover(
    output: &TuneOutputSet,
) -> Result<Vec<AdvisoryLock>, PublicationError> {
    let intended = output
        .destinations()
        .iter()
        .map(|destination| destination.destination_id)
        .collect::<BTreeSet<_>>();
    for _ in 0..8 {
        let before = scan_journals(output.parent())?;
        let closure = closure_for(&intended, &before);
        let locks = acquire_ids(output, closure.iter().copied().collect())?;
        let after = scan_journals(output.parent())?;
        let rescanned = closure_for(&intended, &after);
        if closure != rescanned {
            drop(locks);
            continue;
        }
        let mut set_ids = after
            .iter()
            .filter(|(_, files)| journal_ids(files).iter().any(|id| closure.contains(id)))
            .map(|(set_id, _)| *set_id)
            .collect::<Vec<_>>();
        set_ids.sort();
        for set_id in set_ids {
            let files = after
                .get(&set_id)
                .ok_or(PublicationError::Identity("journal scan changed"))?;
            recover_files(output.parent(), files)?;
        }
        cleanup_journal_free_orphans(output)?;
        return Ok(locks);
    }
    Err(PublicationError::Identity(
        "journal overlap closure did not stabilize",
    ))
}

impl PublicationSet {
    /// Publishes a verified decision and its exact artifact roles primary-last.
    pub fn publish_verified(
        &mut self,
        decision: &super::super::TuneDecision,
        artifacts: TunePublishArtifacts,
    ) -> Result<(), PublicationError> {
        match self.publish_inner(decision, artifacts, None) {
            Ok(()) => Ok(()),
            Err(error) => {
                let recovery = recover_set_id(self.output.parent(), self.output.set_id());
                match recovery.and_then(|()| cleanup_journal_free_orphans(&self.output)) {
                    Ok(()) => Err(error),
                    Err(recovery_error) => Err(PublicationError::Io(format!(
                        "{error}; recovery failed: {recovery_error}"
                    ))),
                }
            }
        }
    }

    /// Executes one publication with a deterministic process-crash injection.
    pub fn publish_with_fault(
        &mut self,
        decision: &super::super::TuneDecision,
        artifacts: TunePublishArtifacts,
        fault: PublicationFault,
    ) -> Result<(), PublicationError> {
        self.publish_inner(decision, artifacts, Some(fault))
    }

    fn publish_inner(
        &mut self,
        decision: &super::super::TuneDecision,
        artifacts: TunePublishArtifacts,
        fault: Option<PublicationFault>,
    ) -> Result<(), PublicationError> {
        revalidate_namespace(&self.output)?;
        let transaction_id = random_transaction_id()?;
        let decision_bytes = super::super::encode_tune_decision(decision);
        let mut journal = PublicationJournal::prepared(
            &self.output,
            transaction_id,
            &decision_bytes,
            &artifacts,
        )?;
        for destination in journal.destinations() {
            let bytes = bytes_for_role(destination.role, &decision_bytes, &artifacts)?;
            let stage = self.output.parent().join(&destination.stage_basename);
            let mut file = create_private(&stage)?;
            file.write_all(bytes)?;
            if destination.role == PublicationRole::Primary && self.output.primary_is_executable() {
                make_executable(&file)?;
            }
            file.sync_all()?;
        }
        sync_directory(self.output.parent())?;
        crash_if(fault, PublicationFault::AfterStages, "stages")?;

        install_journal(self.output.parent(), &journal, fault)?;
        crash_if(
            fault,
            PublicationFault::AfterPhase(JournalPhase::Prepared),
            "prepared",
        )?;

        for destination in journal.destinations() {
            if destination.old.present {
                let backup = self.output.parent().join(&destination.backup_basename);
                if backup.exists() {
                    return Err(PublicationError::Identity("preexisting publication backup"));
                }
                if !atomic_no_replace(&destination.path, &backup)? {
                    return Err(PublicationError::Identity("preexisting publication backup"));
                }
            }
        }
        sync_directory(self.output.parent())?;
        crash_if(fault, PublicationFault::AfterBackups, "backups")?;
        journal = journal.advance(JournalPhase::BackedUp)?;
        install_journal(self.output.parent(), &journal, fault)?;
        crash_if(
            fault,
            PublicationFault::AfterPhase(JournalPhase::BackedUp),
            "backed-up",
        )?;

        publish_role(self.output.parent(), &journal, PublicationRole::Decision)?;
        crash_if(fault, PublicationFault::AfterDecision, "decision")?;
        journal = journal.advance(JournalPhase::DecisionPublished)?;
        install_journal(self.output.parent(), &journal, fault)?;
        crash_if(
            fault,
            PublicationFault::AfterPhase(JournalPhase::DecisionPublished),
            "decision-published",
        )?;

        for role in [PublicationRole::Header, PublicationRole::ImportLibrary] {
            if journal
                .destinations()
                .iter()
                .any(|destination| destination.role == role)
            {
                publish_role(self.output.parent(), &journal, role)?;
            }
        }
        sync_directory(self.output.parent())?;
        crash_if(fault, PublicationFault::AfterSidecars, "sidecars")?;
        journal = journal.advance(JournalPhase::SidecarsPublished)?;
        install_journal(self.output.parent(), &journal, fault)?;
        crash_if(
            fault,
            PublicationFault::AfterPhase(JournalPhase::SidecarsPublished),
            "sidecars-published",
        )?;

        publish_role(self.output.parent(), &journal, PublicationRole::Primary)?;
        crash_if(fault, PublicationFault::AfterPrimary, "primary")?;
        journal = journal.advance(JournalPhase::PrimaryPublished)?;
        install_journal(self.output.parent(), &journal, fault)?;
        crash_if(
            fault,
            PublicationFault::AfterPhase(JournalPhase::PrimaryPublished),
            "primary-published",
        )?;

        verify_complete(&journal, true)?;
        journal = journal.advance(JournalPhase::Committed)?;
        install_journal(self.output.parent(), &journal, fault)?;
        crash_if(
            fault,
            PublicationFault::AfterPhase(JournalPhase::Committed),
            "committed",
        )?;
        cleanup_completed(self.output.parent(), &journal)?;
        Ok(())
    }
}

fn revalidate_namespace(output: &TuneOutputSet) -> Result<(), PublicationError> {
    let decision = output
        .destinations()
        .iter()
        .find(|destination| destination.role == PublicationRole::Decision)
        .ok_or(PublicationError::Identity("decision destination"))?;
    let primary = output
        .destinations()
        .iter()
        .find(|destination| destination.role == PublicationRole::Primary)
        .ok_or(PublicationError::Identity("primary destination"))?;
    let paths = super::TuneArtifactPaths {
        primary: primary.path.clone(),
        header: output
            .destinations()
            .iter()
            .find(|destination| destination.role == PublicationRole::Header)
            .map(|destination| destination.path.clone()),
        import_library: output
            .destinations()
            .iter()
            .find(|destination| destination.role == PublicationRole::ImportLibrary)
            .map(|destination| destination.path.clone()),
    };
    let resolved = TuneOutputSet::resolve(&paths, &decision.path, &[])?;
    if !resolved.same_namespace(output) {
        return Err(PublicationError::Identity(
            "destination namespace changed after locking",
        ));
    }
    Ok(())
}

fn bytes_for_role<'a>(
    role: PublicationRole,
    decision: &'a [u8],
    artifacts: &'a TunePublishArtifacts,
) -> Result<&'a [u8], PublicationError> {
    match role {
        PublicationRole::Decision => Ok(decision),
        PublicationRole::Primary => Ok(&artifacts.primary),
        PublicationRole::Header => artifacts
            .header
            .as_deref()
            .ok_or(PublicationError::Identity("header bytes")),
        PublicationRole::ImportLibrary => artifacts
            .import_library
            .as_deref()
            .ok_or(PublicationError::Identity("import bytes")),
    }
}

fn publish_role(
    parent: &Path,
    journal: &PublicationJournal,
    role: PublicationRole,
) -> Result<(), PublicationError> {
    let destination = journal
        .destinations()
        .iter()
        .find(|destination| destination.role == role)
        .ok_or(PublicationError::Identity("publication role"))?;
    let stage = parent.join(&destination.stage_basename);
    if !atomic_no_replace(&stage, &destination.path)? {
        return Err(PublicationError::Identity(
            "publication destination became occupied",
        ));
    }
    let file = open_private_nofollow(&destination.path)?;
    file.sync_all()?;
    sync_directory(parent)
}

fn install_journal(
    parent: &Path,
    journal: &PublicationJournal,
    fault: Option<PublicationFault>,
) -> Result<(), PublicationError> {
    let set_hex = hex(&journal.set_id());
    let tx_hex = hex(&journal.transaction_id());
    let private = parent.join(format!(
        ".ckc-tune-set-{set_hex}.{tx_hex}.{}.write",
        journal.generation()
    ));
    let update = parent.join(format!(".ckc-tune-set-{set_hex}.journal.new"));
    let active = parent.join(format!(".ckc-tune-set-{set_hex}.journal"));
    let bytes = encode_publication_journal(journal)?;
    let mut file = create_private(&private)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    let decoded = decode_publication_journal(&read_private_nofollow(&private, MAX_JOURNAL_BYTES)?)?;
    if decoded != *journal {
        return Err(PublicationError::Identity("private journal verification"));
    }
    crash_if(
        fault,
        PublicationFault::AfterJournalPrivate(journal.phase()),
        "journal-private",
    )?;
    if !atomic_no_replace(&private, &update)? {
        return Err(PublicationError::Identity("journal update already exists"));
    }
    sync_directory(parent)?;
    crash_if(
        fault,
        PublicationFault::AfterJournalUpdate(journal.phase()),
        "journal-update",
    )?;
    if active.exists() {
        atomic_replace(&update, &active)?;
    } else if !atomic_no_replace(&update, &active)? {
        return Err(PublicationError::Identity("journal active race"));
    }
    sync_directory(parent)
}

fn crash_if(
    configured: Option<PublicationFault>,
    point: PublicationFault,
    label: &'static str,
) -> Result<(), PublicationError> {
    if configured == Some(point) {
        Err(PublicationError::InjectedCrash(label))
    } else {
        Ok(())
    }
}

fn scan_journals(parent: &Path) -> Result<BTreeMap<[u8; 32], JournalFiles>, PublicationError> {
    let mut groups = BTreeMap::<[u8; 32], JournalFiles>::new();
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| PublicationError::Identity("non-UTF-8 reserved filename"))?;
        let Some((set_id, update)) = journal_name(&name)? else {
            continue;
        };
        let bytes = read_private_nofollow(&entry.path(), MAX_JOURNAL_BYTES)?;
        let journal = decode_publication_journal(&bytes)?;
        if journal.set_id() != set_id {
            return Err(PublicationError::Identity("journal filename identity"));
        }
        journal.revalidate_paths()?;
        let group = groups.entry(set_id).or_insert(JournalFiles {
            active: None,
            update: None,
        });
        let slot = if update {
            &mut group.update
        } else {
            &mut group.active
        };
        if slot.replace((entry.path(), journal)).is_some() {
            return Err(PublicationError::Identity("duplicate journal final name"));
        }
    }
    Ok(groups)
}

fn journal_name(name: &str) -> Result<Option<([u8; 32], bool)>, PublicationError> {
    let Some(rest) = name.strip_prefix(".ckc-tune-set-") else {
        return Ok(None);
    };
    let (hex_value, update) = if let Some(value) = rest.strip_suffix(".journal.new") {
        (value, true)
    } else if let Some(value) = rest.strip_suffix(".journal") {
        (value, false)
    } else {
        return Ok(None);
    };
    if hex_value.len() != 64 || !hex_value.bytes().all(is_lower_hex) {
        return Err(PublicationError::Identity(
            "malformed reserved journal name",
        ));
    }
    Ok(Some((decode_hex_32(hex_value)?, update)))
}

fn closure_for(
    intended: &BTreeSet<[u8; 32]>,
    groups: &BTreeMap<[u8; 32], JournalFiles>,
) -> BTreeSet<[u8; 32]> {
    let mut closure = intended.clone();
    loop {
        let mut changed = false;
        for files in groups.values() {
            let ids = journal_ids(files);
            if ids.iter().any(|id| closure.contains(id)) {
                for id in ids {
                    changed |= closure.insert(id);
                }
            }
        }
        if !changed {
            return closure;
        }
    }
}

fn journal_ids(files: &JournalFiles) -> BTreeSet<[u8; 32]> {
    files
        .active
        .as_ref()
        .or(files.update.as_ref())
        .into_iter()
        .flat_map(|(_, journal)| journal.destinations())
        .map(|destination| destination.destination_id)
        .collect()
}

fn recover_set_id(parent: &Path, set_id: [u8; 32]) -> Result<(), PublicationError> {
    let groups = scan_journals(parent)?;
    if let Some(files) = groups.get(&set_id) {
        recover_files(parent, files)?;
    }
    Ok(())
}

fn recover_files(parent: &Path, files: &JournalFiles) -> Result<(), PublicationError> {
    let journal = resolve_metadata_table(parent, files)?;
    validate_present_identities(parent, &journal)?;
    let primary = journal
        .destinations()
        .iter()
        .find(|destination| destination.role == PublicationRole::Primary)
        .ok_or(PublicationError::Identity("journal primary"))?;
    let primary_actual = identity_at(&primary.path)?;
    let distinct_primary = primary.old != primary.new;
    let roll_forward = journal.direction() == RecoveryDirection::Forward
        && (journal.phase() >= JournalPhase::PrimaryPublished
            || (journal.phase() < JournalPhase::PrimaryPublished
                && distinct_primary
                && primary_actual == primary.new));
    if roll_forward {
        roll_forward_set(parent, journal)
    } else {
        let journal = if journal.direction() == RecoveryDirection::Forward {
            let rollback = journal.begin_rollback()?;
            install_journal(parent, &rollback, None)?;
            rollback
        } else {
            journal
        };
        rollback_set(parent, journal)
    }
}

fn resolve_metadata_table(
    parent: &Path,
    files: &JournalFiles,
) -> Result<PublicationJournal, PublicationError> {
    match (&files.active, &files.update) {
        (Some((_, active)), None) => {
            remove_private_writes(parent, active.set_id())?;
            Ok(active.clone())
        }
        (None, Some((update_path, update)))
            if update.direction() == RecoveryDirection::Forward
                && update.phase() == JournalPhase::Prepared
                && update.generation() == 1 =>
        {
            let active_path = active_path(parent, update.set_id());
            if !atomic_no_replace(update_path, &active_path)? {
                return Err(PublicationError::Identity("journal promotion race"));
            }
            sync_directory(parent)?;
            remove_private_writes(parent, update.set_id())?;
            Ok(update.clone())
        }
        (Some((active_path, active)), Some((update_path, update)))
            if update.is_successor_of(active) =>
        {
            atomic_replace(update_path, active_path)?;
            sync_directory(parent)?;
            remove_private_writes(parent, active.set_id())?;
            Ok(update.clone())
        }
        (None, None) => Err(PublicationError::Identity("empty journal metadata table")),
        _ => Err(PublicationError::Identity(
            "journal update is not a valid successor",
        )),
    }
}

fn validate_present_identities(
    parent: &Path,
    journal: &PublicationJournal,
) -> Result<(), PublicationError> {
    for destination in journal.destinations() {
        let actual = identity_at(&destination.path)?;
        if actual.present && actual != destination.old && actual != destination.new {
            return Err(PublicationError::Identity("destination has third identity"));
        }
        let stage = identity_at(&parent.join(&destination.stage_basename))?;
        if stage.present && stage != destination.new {
            return Err(PublicationError::Identity("stage has third identity"));
        }
        let backup = identity_at(&parent.join(&destination.backup_basename))?;
        if backup.present && (!destination.old.present || backup != destination.old) {
            return Err(PublicationError::Identity("backup has third identity"));
        }
    }
    Ok(())
}

fn rollback_set(parent: &Path, journal: PublicationJournal) -> Result<(), PublicationError> {
    for destination in journal.destinations().iter().rev() {
        let actual = identity_at(&destination.path)?;
        let backup_path = parent.join(&destination.backup_basename);
        let backup = identity_at(&backup_path)?;
        if destination.old.present {
            if actual == destination.old {
                remove_matching(&backup_path, destination.old)?;
            } else if backup == destination.old {
                if actual.present {
                    if actual != destination.new {
                        return Err(PublicationError::Identity("rollback destination identity"));
                    }
                    fs::remove_file(&destination.path)?;
                }
                if !atomic_no_replace(&backup_path, &destination.path)? {
                    return Err(PublicationError::Identity(
                        "rollback destination became occupied",
                    ));
                }
            } else {
                return Err(PublicationError::Identity("missing sole old copy"));
            }
        } else {
            if backup.present {
                return Err(PublicationError::Identity("backup for absent old output"));
            }
            if actual.present {
                if actual != destination.new {
                    return Err(PublicationError::Identity("rollback new identity"));
                }
                fs::remove_file(&destination.path)?;
            }
        }
        remove_matching(&parent.join(&destination.stage_basename), destination.new)?;
    }
    sync_directory(parent)?;
    verify_complete(&journal, false)?;
    cleanup_journal_names(parent, &journal)
}

fn roll_forward_set(
    parent: &Path,
    mut journal: PublicationJournal,
) -> Result<(), PublicationError> {
    if journal.phase() < JournalPhase::BackedUp {
        ensure_backups(parent, &journal)?;
        journal = journal.advance(JournalPhase::BackedUp)?;
        install_journal(parent, &journal, None)?;
    }
    ensure_new_role(parent, &journal, PublicationRole::Decision)?;
    if journal.phase() < JournalPhase::DecisionPublished {
        journal = journal.advance(JournalPhase::DecisionPublished)?;
        install_journal(parent, &journal, None)?;
    }
    for role in [PublicationRole::Header, PublicationRole::ImportLibrary] {
        if journal
            .destinations()
            .iter()
            .any(|destination| destination.role == role)
        {
            ensure_new_role(parent, &journal, role)?;
        }
    }
    sync_directory(parent)?;
    if journal.phase() < JournalPhase::SidecarsPublished {
        journal = journal.advance(JournalPhase::SidecarsPublished)?;
        install_journal(parent, &journal, None)?;
    }
    ensure_new_role(parent, &journal, PublicationRole::Primary)?;
    if journal.phase() < JournalPhase::PrimaryPublished {
        journal = journal.advance(JournalPhase::PrimaryPublished)?;
        install_journal(parent, &journal, None)?;
    }
    verify_complete(&journal, true)?;
    if journal.phase() < JournalPhase::Committed {
        journal = journal.advance(JournalPhase::Committed)?;
        install_journal(parent, &journal, None)?;
    }
    cleanup_completed(parent, &journal)
}

fn ensure_backups(parent: &Path, journal: &PublicationJournal) -> Result<(), PublicationError> {
    for destination in journal.destinations() {
        if destination.old.present {
            let backup_path = parent.join(&destination.backup_basename);
            let backup = identity_at(&backup_path)?;
            if backup == destination.old {
                continue;
            }
            let actual = identity_at(&destination.path)?;
            if actual != destination.old {
                return Err(PublicationError::Identity("missing old output for backup"));
            }
            if !atomic_no_replace(&destination.path, &backup_path)? {
                return Err(PublicationError::Identity(
                    "recovery backup became occupied",
                ));
            }
        }
    }
    sync_directory(parent)
}

fn ensure_new_role(
    parent: &Path,
    journal: &PublicationJournal,
    role: PublicationRole,
) -> Result<(), PublicationError> {
    let destination = journal
        .destinations()
        .iter()
        .find(|destination| destination.role == role)
        .ok_or(PublicationError::Identity("recovery role"))?;
    if identity_at(&destination.path)? == destination.new {
        return Ok(());
    }
    let stage = parent.join(&destination.stage_basename);
    if identity_at(&stage)? != destination.new {
        return Err(PublicationError::Identity("missing sole new copy"));
    }
    if identity_at(&destination.path)?.present {
        return Err(PublicationError::Identity(
            "destination occupied during roll-forward",
        ));
    }
    if !atomic_no_replace(&stage, &destination.path)? {
        return Err(PublicationError::Identity(
            "roll-forward destination became occupied",
        ));
    }
    let file = open_private_nofollow(&destination.path)?;
    file.sync_all()?;
    sync_directory(parent)
}

fn verify_complete(journal: &PublicationJournal, expect_new: bool) -> Result<(), PublicationError> {
    for destination in journal.destinations() {
        let expected = if expect_new {
            destination.new
        } else {
            destination.old
        };
        if identity_at(&destination.path)? != expected {
            return Err(PublicationError::Identity(
                "incomplete recovered output set",
            ));
        }
    }
    Ok(())
}

fn cleanup_completed(parent: &Path, journal: &PublicationJournal) -> Result<(), PublicationError> {
    for destination in journal.destinations() {
        remove_matching(&parent.join(&destination.stage_basename), destination.new)?;
        remove_matching(&parent.join(&destination.backup_basename), destination.old)?;
    }
    sync_directory(parent)?;
    cleanup_journal_names(parent, journal)
}

fn cleanup_journal_names(
    parent: &Path,
    journal: &PublicationJournal,
) -> Result<(), PublicationError> {
    remove_if_exists(&active_path(parent, journal.set_id()))?;
    remove_if_exists(&update_path(parent, journal.set_id()))?;
    remove_private_writes(parent, journal.set_id())?;
    sync_directory(parent)
}

fn remove_matching(path: &Path, expected: ContentIdentity) -> Result<(), PublicationError> {
    let actual = identity_at(path)?;
    if !actual.present {
        return Ok(());
    }
    if !expected.present || actual != expected {
        return Err(PublicationError::Identity("cleanup identity mismatch"));
    }
    fs::remove_file(path)?;
    Ok(())
}

fn cleanup_journal_free_orphans(output: &TuneOutputSet) -> Result<(), PublicationError> {
    let set_hex = hex(&output.set_id());
    let prefix = format!(".ckc-tune-set-{set_hex}.");
    for entry in fs::read_dir(output.parent())? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| PublicationError::Identity("non-UTF-8 reserved name"))?;
        if !name.starts_with(&prefix) {
            continue;
        }
        if name.ends_with(".journal") || name.ends_with(".journal.new") {
            return Err(PublicationError::Identity("unrecovered intended journal"));
        }
        if name.ends_with(".backup") {
            return Err(PublicationError::Identity("journal-free backup"));
        }
        if valid_orphan_stage_or_write(&name, &prefix) {
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(PublicationError::Identity("unsafe orphan"));
            }
            fs::remove_file(entry.path())?;
        } else {
            return Err(PublicationError::Identity("malformed intended orphan"));
        }
    }
    sync_directory(output.parent())
}

fn valid_orphan_stage_or_write(name: &str, prefix: &str) -> bool {
    let rest = &name[prefix.len()..];
    let mut parts = rest.split('.');
    let Some(tx) = parts.next() else {
        return false;
    };
    if tx.len() != 32 || !tx.bytes().all(is_lower_hex) {
        return false;
    }
    let Some(kind) = parts.next() else {
        return false;
    };
    let Some(suffix) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    (matches!(kind, "decision" | "header" | "import" | "primary") && suffix == "stage")
        || (suffix == "write"
            && kind
                .parse::<u64>()
                .is_ok_and(|value| value > 0 && value.to_string() == kind))
}

fn remove_private_writes(parent: &Path, set_id: [u8; 32]) -> Result<(), PublicationError> {
    let prefix = format!(".ckc-tune-set-{}.", hex(&set_id));
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| PublicationError::Identity("non-UTF-8 reserved name"))?;
        if !name.starts_with(&prefix) || !name.ends_with(".write") {
            continue;
        }
        if !valid_private_write(&name, &prefix) {
            return Err(PublicationError::Identity(
                "malformed private journal write",
            ));
        }
        let file = open_private_nofollow(&entry.path())?;
        drop(file);
        fs::remove_file(entry.path())?;
    }
    sync_directory(parent)
}

fn valid_private_write(name: &str, prefix: &str) -> bool {
    let rest = &name[prefix.len()..];
    let mut parts = rest.split('.');
    let Some(tx) = parts.next() else {
        return false;
    };
    let Some(generation) = parts.next() else {
        return false;
    };
    let Some(suffix) = parts.next() else {
        return false;
    };
    if parts.next().is_some()
        || tx.len() != 32
        || !tx.bytes().all(is_lower_hex)
        || suffix != "write"
    {
        return false;
    }
    generation
        .parse::<u64>()
        .is_ok_and(|value| value > 0 && value.to_string() == generation)
}

fn active_path(parent: &Path, set_id: [u8; 32]) -> PathBuf {
    parent.join(format!(".ckc-tune-set-{}.journal", hex(&set_id)))
}

fn update_path(parent: &Path, set_id: [u8; 32]) -> PathBuf {
    parent.join(format!(".ckc-tune-set-{}.journal.new", hex(&set_id)))
}

fn remove_if_exists(path: &Path) -> Result<(), PublicationError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn decode_hex_32(value: &str) -> Result<[u8; 32], PublicationError> {
    let mut output = [0u8; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        output[index] = (hex_nibble(chunk[0])? << 4) | hex_nibble(chunk[1])?;
    }
    Ok(output)
}

fn hex_nibble(value: u8) -> Result<u8, PublicationError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(PublicationError::Identity("hex digit")),
    }
}

fn is_lower_hex(value: u8) -> bool {
    value.is_ascii_digit() || matches!(value, b'a'..=b'f')
}
