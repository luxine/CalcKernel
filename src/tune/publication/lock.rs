use std::{fs, io::Write};

use super::{
    PublicationError, TuneOutputSet,
    platform::{
        AdvisoryLock, atomic_no_replace, create_private, open_private_nofollow,
        random_transaction_id, sync_directory,
    },
};

const LOCK_MAGIC: &[u8; 8] = b"CKTLCK01";

/// A stable output-set closure held under persistent destination locks.
pub struct PublicationSet {
    pub(crate) output: TuneOutputSet,
    pub(crate) locks: Vec<AdvisoryLock>,
}

impl PublicationSet {
    /// Acquires every intended destination lock in full-id order.
    ///
    /// Journal overlap expansion and recovery are added by the journal layer
    /// before this constructor is admitted to publication.
    pub fn acquire_and_recover(output: TuneOutputSet) -> Result<Self, PublicationError> {
        let locks = super::recovery::acquire_stable_closure_and_recover(&output)?;
        Ok(Self { output, locks })
    }

    #[must_use]
    pub const fn output_set(&self) -> &TuneOutputSet {
        &self.output
    }
}

pub(crate) fn acquire_ids(
    output: &TuneOutputSet,
    mut ids: Vec<[u8; 32]>,
) -> Result<Vec<AdvisoryLock>, PublicationError> {
    ids.sort();
    ids.dedup();
    let mut locks = Vec::with_capacity(ids.len());
    for id in ids {
        locks.push(acquire_one(output, id)?);
    }
    Ok(locks)
}

impl Drop for PublicationSet {
    fn drop(&mut self) {
        while self.locks.pop().is_some() {}
    }
}

fn acquire_one(output: &TuneOutputSet, id: [u8; 32]) -> Result<AdvisoryLock, PublicationError> {
    let hex = hex(&id);
    let final_path = output.parent().join(format!(".ckc-tune-dest-{hex}.lock"));
    if !final_path.exists() {
        initialize_lock(output, id, &final_path, &hex)?;
    }
    let mut file = open_private_nofollow(&final_path)?;
    let mut expected = Vec::with_capacity(40);
    expected.extend_from_slice(LOCK_MAGIC);
    expected.extend_from_slice(&id);
    let mut actual = Vec::new();
    use std::io::{Read, Seek};
    file.rewind()?;
    file.read_to_end(&mut actual)?;
    if actual != expected {
        return Err(PublicationError::Identity("persistent lock identity"));
    }
    AdvisoryLock::acquire(file)
}

fn initialize_lock(
    output: &TuneOutputSet,
    id: [u8; 32],
    final_path: &std::path::Path,
    id_hex: &str,
) -> Result<(), PublicationError> {
    let tx = hex(&random_transaction_id()?);
    let initializer = output
        .parent()
        .join(format!(".ckc-tune-lock-init-{id_hex}.{tx}.write"));
    let result = (|| {
        let mut file = create_private(&initializer)?;
        file.write_all(LOCK_MAGIC)?;
        file.write_all(&id)?;
        file.sync_all()?;
        drop(file);
        let installed = atomic_no_replace(&initializer, final_path)?;
        if !installed {
            remove_if_exists(&initializer)?;
        }
        sync_directory(output.parent())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = remove_if_exists(&initializer);
    }
    result
}

fn remove_if_exists(path: &std::path::Path) -> Result<(), PublicationError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
