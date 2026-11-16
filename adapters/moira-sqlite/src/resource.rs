//! Shared local ownership, not a second SQLite locking protocol.
use crate::{StoreError, io};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

#[doc(hidden)]
pub struct OwnedResource {
    owner_file: File,
    resource_file: File,
    pub lock_path: PathBuf,
    lock_identity: (u64, u64),
    path: PathBuf,
    identity: (u64, u64),
}
impl OwnedResource {
    pub fn open(path: impl AsRef<Path>, max_pages: u32) -> Result<Self, StoreError> {
        if max_pages < 4 || max_pages == u32::MAX {
            return Err(StoreError::InvalidConfig);
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path.as_ref())
            .map_err(io)?;
        let metadata = file.metadata().map_err(io)?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.len() > u64::from(max_pages) * 4096
        {
            return Err(StoreError::InvalidConfig);
        }
        let identity = (metadata.dev(), metadata.ino());
        let path = fs::canonicalize(path).map_err(io)?;
        // macOS whole-file flock must not be mixed with SQLite database locks.
        // The sidecar persists across owner lifetimes. Never unlink it in Drop.
        let mut name = path.as_os_str().to_os_string();
        name.push(".owner.lock");
        let lock_path = PathBuf::from(name);
        let owner_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&lock_path)
            .map_err(io)?;
        owner_file.try_lock().map_err(|e| match e {
            TryLockError::WouldBlock => StoreError::Busy,
            TryLockError::Error(e) => io(e),
        })?;
        let lock = owner_file.metadata().map_err(io)?;
        if !lock.is_file() || lock.nlink() != 1 {
            return Err(StoreError::InvalidConfig);
        }
        let result = Self {
            owner_file,
            resource_file: file,
            lock_path,
            lock_identity: (lock.dev(), lock.ino()),
            path,
            identity,
        };
        result.check()?;
        Ok(result)
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn sync_parent(&self) -> Result<(), StoreError> {
        File::open(self.path.parent().ok_or(StoreError::InvalidConfig)?)
            .map_err(io)?
            .sync_all()
            .map_err(io)
    }
    pub fn check(&self) -> Result<(), StoreError> {
        let path = fs::metadata(&self.path).map_err(io)?;
        let file = self.resource_file.metadata().map_err(io)?;
        let lock_path = fs::metadata(&self.lock_path).map_err(io)?;
        let lock = self.owner_file.metadata().map_err(io)?;
        if (path.dev(), path.ino()) != self.identity
            || (file.dev(), file.ino()) != self.identity
            || path.nlink() != 1
            || (lock_path.dev(), lock_path.ino()) != self.lock_identity
            || (lock.dev(), lock.ino()) != self.lock_identity
            || lock_path.nlink() != 1
        {
            return Err(StoreError::LostResource);
        }
        Ok(())
    }
}
