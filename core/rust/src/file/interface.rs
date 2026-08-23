//! Provider-neutral mounted filesystem interface.
//!
//! `std.native.File` remains promise-based at the Hara boundary. Providers use
//! typed local futures here so browser implementations may retain non-`Send`
//! host handles while native implementations schedule blocking I/O elsewhere.

#[path = "legacy.rs"]
mod legacy;

use self::legacy::LegacyFilesystem;
use crate::file::{
    CopyOptions, DeleteOptions, FileEntry, FileError, FileProvider, FileType, MkdirOptions,
    MoveOptions, WriteOptions,
};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

pub type FilesystemFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, FileError>> + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FilesystemCapability {
    Read,
    Write,
    Entries,
    Mkdir,
    Delete,
    Copy,
    Move,
    Append,
    AtomicMove,
    PreserveModified,
    RevisionCheck,
    Transactions,
    Watch,
}

impl FilesystemCapability {
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Entries => "entries",
            Self::Mkdir => "mkdir",
            Self::Delete => "delete",
            Self::Copy => "copy",
            Self::Move => "move",
            Self::Append => "append",
            Self::AtomicMove => "atomic-move",
            Self::PreserveModified => "preserve-modified",
            Self::RevisionCheck => "revision-check",
            Self::Transactions => "transactions",
            Self::Watch => "watch",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FilesystemCapabilities {
    values: BTreeSet<FilesystemCapability>,
}

impl FilesystemCapabilities {
    pub fn new(values: impl IntoIterator<Item = FilesystemCapability>) -> Self {
        Self {
            values: values.into_iter().collect(),
        }
    }

    pub fn read_only() -> Self {
        Self::new([FilesystemCapability::Read, FilesystemCapability::Entries])
    }

    pub fn legacy_read_write() -> Self {
        Self::new([
            FilesystemCapability::Read,
            FilesystemCapability::Write,
            FilesystemCapability::Entries,
            FilesystemCapability::Mkdir,
            FilesystemCapability::Delete,
            FilesystemCapability::Copy,
            FilesystemCapability::Move,
            FilesystemCapability::Append,
            FilesystemCapability::PreserveModified,
        ])
    }

    pub fn contains(&self, capability: FilesystemCapability) -> bool {
        self.values.contains(&capability)
    }

    pub fn iter(&self) -> impl Iterator<Item = FilesystemCapability> + '_ {
        self.values.iter().copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesystemDescriptor {
    kind: String,
    display: String,
    read_only: bool,
    capabilities: FilesystemCapabilities,
    revision: Option<String>,
    extensions: BTreeMap<String, String>,
}

impl FilesystemDescriptor {
    pub fn new(
        kind: impl Into<String>,
        display: impl Into<String>,
        read_only: bool,
        capabilities: FilesystemCapabilities,
    ) -> Self {
        Self {
            kind: kind.into(),
            display: display.into(),
            read_only,
            capabilities,
            revision: None,
            extensions: BTreeMap::new(),
        }
    }

    pub fn legacy(kind: impl Into<String>, display: impl Into<String>) -> Self {
        Self::new(
            kind,
            display,
            false,
            FilesystemCapabilities::legacy_read_write(),
        )
    }

    pub fn with_revision(mut self, revision: impl Into<String>) -> Self {
        self.revision = Some(revision.into());
        self
    }

    pub fn with_extension(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extensions.insert(key.into(), value.into());
        self
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn display(&self) -> &str {
        &self.display
    }

    pub fn read_only(&self) -> bool {
        self.read_only
    }

    pub fn capabilities(&self) -> &FilesystemCapabilities {
        &self.capabilities
    }

    pub fn revision(&self) -> Option<&str> {
        self.revision.as_deref()
    }

    pub fn extensions(&self) -> &BTreeMap<String, String> {
        &self.extensions
    }
}

#[derive(Debug, Clone)]
pub struct FilesystemCallContext {
    deadline: Option<Instant>,
    cancelled: Arc<AtomicBool>,
    trace_id: Option<Arc<str>>,
}

impl Default for FilesystemCallContext {
    fn default() -> Self {
        Self {
            deadline: None,
            cancelled: Arc::new(AtomicBool::new(false)),
            trace_id: None,
        }
    }
}

impl FilesystemCallContext {
    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    pub fn with_trace_id(mut self, trace_id: impl Into<Arc<str>>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    pub fn trace_id(&self) -> Option<&str> {
        self.trace_id.as_deref()
    }

    pub fn cancel(&self) -> bool {
        !self.cancelled.swap(true, Ordering::AcqRel)
    }

    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn check(&self) -> Result<(), FileError> {
        if self.cancelled() {
            return Err(FileError::Io("filesystem operation cancelled".into()));
        }
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(FileError::Io("filesystem operation timed out".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesystemPageRequest {
    pub token: Option<String>,
    pub limit: usize,
}

impl Default for FilesystemPageRequest {
    fn default() -> Self {
        Self {
            token: None,
            limit: 256,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesystemEntry {
    pub path: String,
    pub name: String,
    pub kind: FileType,
    pub size: Option<u64>,
    pub modified_at: Option<i64>,
    pub id: Option<String>,
    pub revision: Option<String>,
    pub capabilities: Option<FilesystemCapabilities>,
    pub extensions: BTreeMap<String, String>,
}

impl From<FileEntry> for FilesystemEntry {
    fn from(entry: FileEntry) -> Self {
        Self {
            path: entry.path,
            name: entry.name,
            kind: entry.kind,
            size: entry.size,
            modified_at: Some(entry.modified_at),
            id: None,
            revision: None,
            capabilities: None,
            extensions: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesystemEntryPage {
    pub entries: Vec<FilesystemEntry>,
    pub next_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FilesystemMutationContext {
    pub expected_revision: Option<String>,
    pub expected_target_revision: Option<String>,
}

impl FilesystemMutationContext {
    pub fn required(&self) -> bool {
        self.expected_revision.is_some() || self.expected_target_revision.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesystemMutation {
    pub path: String,
    pub revision: Option<String>,
    pub mount_revision: Option<String>,
    pub extensions: BTreeMap<String, String>,
}

impl FilesystemMutation {
    pub fn path(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            revision: None,
            mount_revision: None,
            extensions: BTreeMap::new(),
        }
    }
}

pub trait SynchronousFileProvider: FileProvider {}

pub trait IFilesystem {
    fn descriptor(&self) -> FilesystemDescriptor;

    fn capabilities(&self) -> FilesystemCapabilities {
        self.descriptor().capabilities().clone()
    }

    fn stat<'a>(
        &'a self,
        context: FilesystemCallContext,
        path: String,
    ) -> FilesystemFuture<'a, FilesystemEntry>;

    fn read<'a>(
        &'a self,
        context: FilesystemCallContext,
        path: String,
    ) -> FilesystemFuture<'a, Vec<u8>>;

    fn write<'a>(
        &'a self,
        context: FilesystemCallContext,
        path: String,
        bytes: Vec<u8>,
        options: WriteOptions,
        mutation: FilesystemMutationContext,
    ) -> FilesystemFuture<'a, FilesystemMutation>;

    fn entries_page<'a>(
        &'a self,
        context: FilesystemCallContext,
        path: String,
        request: FilesystemPageRequest,
    ) -> FilesystemFuture<'a, FilesystemEntryPage>;

    fn mkdir<'a>(
        &'a self,
        context: FilesystemCallContext,
        path: String,
        options: MkdirOptions,
        mutation: FilesystemMutationContext,
    ) -> FilesystemFuture<'a, FilesystemMutation>;

    fn delete<'a>(
        &'a self,
        context: FilesystemCallContext,
        path: String,
        options: DeleteOptions,
        mutation: FilesystemMutationContext,
    ) -> FilesystemFuture<'a, FilesystemMutation>;

    fn copy<'a>(
        &'a self,
        context: FilesystemCallContext,
        source: String,
        target: String,
        options: CopyOptions,
        mutation: FilesystemMutationContext,
    ) -> FilesystemFuture<'a, FilesystemMutation>;

    fn move_entry<'a>(
        &'a self,
        context: FilesystemCallContext,
        source: String,
        target: String,
        options: MoveOptions,
        mutation: FilesystemMutationContext,
    ) -> FilesystemFuture<'a, FilesystemMutation>;

    fn close<'a>(&'a self, context: FilesystemCallContext) -> FilesystemFuture<'a, ()>;
}

#[path = "providers.rs"]
pub mod providers;
#[cfg(not(target_arch = "wasm32"))]
pub mod sftp;

#[derive(Clone)]
pub struct FilesystemHandle {
    filesystem: Rc<dyn IFilesystem>,
}

impl std::fmt::Debug for FilesystemHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FilesystemHandle")
            .field("descriptor", &self.descriptor())
            .finish()
    }
}

impl FilesystemHandle {
    pub fn new<F: IFilesystem + 'static>(filesystem: F) -> Self {
        Self {
            filesystem: Rc::new(filesystem),
        }
    }

    pub fn from_legacy<P: SynchronousFileProvider + 'static>(
        provider: P,
        descriptor: FilesystemDescriptor,
    ) -> Self {
        Self::new(LegacyFilesystem::new(provider, descriptor))
    }

    pub fn descriptor(&self) -> FilesystemDescriptor {
        self.filesystem.descriptor()
    }

    pub fn capabilities(&self) -> FilesystemCapabilities {
        self.filesystem.capabilities()
    }

    pub fn as_filesystem(&self) -> &dyn IFilesystem {
        self.filesystem.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_context_cancellation_is_shared() {
        let context = FilesystemCallContext::default();
        let second = context.clone();
        assert!(context.cancel());
        assert!(second.cancelled());
        assert!(matches!(second.check(), Err(FileError::Io(_))));
    }

    #[test]
    fn descriptors_expose_only_explicit_redacted_fields() {
        let descriptor = FilesystemDescriptor::new(
            "github",
            "hara-lang/hara@main",
            false,
            FilesystemCapabilities::new([
                FilesystemCapability::Read,
                FilesystemCapability::Entries,
                FilesystemCapability::RevisionCheck,
            ]),
        )
        .with_revision("commit-sha")
        .with_extension("provider/ref", "heads/main");
        assert_eq!(descriptor.kind(), "github");
        assert_eq!(descriptor.display(), "hara-lang/hara@main");
        assert_eq!(descriptor.revision(), Some("commit-sha"));
        assert!(descriptor
            .capabilities()
            .contains(FilesystemCapability::RevisionCheck));
    }
}
