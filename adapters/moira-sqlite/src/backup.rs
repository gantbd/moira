//! Bounded online snapshot publication for an already-owned SQLite database.

use crate::{OwnedResource, StoreError, validate_sqlite_integrity};
use rusqlite::{Connection, OpenFlags, backup::StepResult};
use std::{
    fs::{self, File, OpenOptions},
    io::ErrorKind,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OnlineBackupLimitsV1 {
    pub pages_per_step: u32,
    pub max_steps: u32,
    pub retry_delay: Duration,
}

impl Default for OnlineBackupLimitsV1 {
    fn default() -> Self {
        Self {
            pages_per_step: 16,
            max_steps: 4_096,
            retry_delay: Duration::from_millis(1),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OnlineBackupReceiptV1 {
    pub storage_revision: Option<crate::StoreRevision>,
    pub page_count: u32,
}

struct PendingBackup {
    file: File,
    path: PathBuf,
    identity: (u64, u64),
    published: bool,
}

impl PendingBackup {
    fn create(path: PathBuf) -> Result<Self, StoreError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|error| match error.kind() {
                ErrorKind::AlreadyExists => StoreError::BackupExists,
                _ => super::io(error),
            })?;
        let metadata = file.metadata().map_err(super::io)?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(StoreError::InvalidConfig);
        }
        Ok(Self {
            file,
            path,
            identity: (metadata.dev(), metadata.ino()),
            published: false,
        })
    }

    fn check(&self) -> Result<(), StoreError> {
        let path = fs::metadata(&self.path).map_err(super::io)?;
        let file = self.file.metadata().map_err(super::io)?;
        if (path.dev(), path.ino()) != self.identity
            || (file.dev(), file.ino()) != self.identity
            || path.nlink() != 1
        {
            return Err(StoreError::LostResource);
        }
        Ok(())
    }
}

impl Drop for PendingBackup {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub(crate) fn publish_online_backup(
    source: &Connection,
    resource: &OwnedResource,
    destination: &Path,
    source_max_pages: u32,
    limits: OnlineBackupLimitsV1,
) -> Result<u32, StoreError> {
    if limits.pages_per_step == 0
        || limits.pages_per_step > 4_096
        || limits.max_steps == 0
        || limits.max_steps > 1_000_000
        || limits.retry_delay > Duration::from_secs(1)
    {
        return Err(StoreError::InvalidConfig);
    }
    resource.check()?;
    match fs::symlink_metadata(destination) {
        Ok(_) => return Err(StoreError::BackupExists),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(super::io(error)),
    }
    let parent = destination.parent().ok_or(StoreError::InvalidConfig)?;
    let file_name = destination
        .file_name()
        .ok_or(StoreError::InvalidConfig)?
        .to_os_string();
    let mut partial_name = file_name;
    partial_name.push(".partial");
    let partial_path = parent.join(partial_name);
    let mut pending = PendingBackup::create(partial_path)?;
    let canonical = fs::canonicalize(&pending.path).map_err(super::io)?;
    let mut target = Connection::open_with_flags(
        &canonical,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    target.busy_timeout(Duration::ZERO)?;
    let backup = rusqlite::backup::Backup::new(source, &mut target)?;
    let pages_per_step =
        i32::try_from(limits.pages_per_step).map_err(|_| StoreError::InvalidConfig)?;
    let mut page_count = None;
    for _ in 0..limits.max_steps {
        match backup.step(pages_per_step)? {
            StepResult::Done => {
                page_count = Some(
                    u32::try_from(backup.progress().pagecount)
                        .map_err(|_| StoreError::BackupIncomplete)?,
                );
                break;
            }
            StepResult::More | StepResult::Busy | StepResult::Locked => {
                if !limits.retry_delay.is_zero() {
                    thread::sleep(limits.retry_delay);
                }
            }
            _ => return Err(StoreError::BackupIncomplete),
        }
    }
    let page_count = page_count.ok_or(StoreError::BackupIncomplete)?;
    if page_count > source_max_pages {
        return Err(StoreError::BackupIncomplete);
    }
    drop(backup);
    drop(target);
    pending.check()?;
    let verification = Connection::open_with_flags(
        &canonical,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    validate_sqlite_integrity(&verification)?;
    drop(verification);
    resource.check()?;
    pending.file.sync_all().map_err(super::io)?;
    fs::rename(&pending.path, destination).map_err(super::io)?;
    // The destination is visible after rename. From this point onward a failure
    // must not be reported as an ordinary I/O refusal: the caller must inspect
    // the destination instead of retrying and potentially publishing twice.
    pending.published = true;
    File::open(parent)
        .map_err(super::io)?
        .sync_all()
        .map_err(|_| StoreError::BackupPublicationUncertain)?;
    Ok(page_count)
}
