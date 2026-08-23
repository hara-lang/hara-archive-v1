//! Migration adapter from the synchronous `FileProvider` boundary.
//!
//! The adapter intentionally performs the legacy operation when its future is
//! polled. Native disk migration must replace this with an I/O-scheduler-backed
//! implementation before `FileProvider` is removed.

use super::{
    FilesystemCallContext, FilesystemDescriptor, FilesystemEntry, FilesystemEntryPage,
    FilesystemFuture, FilesystemMutation, FilesystemMutationContext, FilesystemPageRequest,
    IFilesystem, SynchronousFileProvider,
};
use crate::file::{CopyOptions, DeleteOptions, FileError, MkdirOptions, MoveOptions, WriteOptions};
use std::cell::Cell;
use std::rc::Rc;

#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
impl SynchronousFileProvider for crate::file::NativeFileProvider {}
impl SynchronousFileProvider for crate::file::MemoryFileProvider {}
impl SynchronousFileProvider for crate::file::UnsupportedFileProvider {}

pub(super) struct LegacyFilesystem<P> {
    provider: P,
    descriptor: FilesystemDescriptor,
    closed: Rc<Cell<bool>>,
}

impl<P> LegacyFilesystem<P> {
    pub fn new(provider: P, descriptor: FilesystemDescriptor) -> Self {
        Self {
            provider,
            descriptor,
            closed: Rc::new(Cell::new(false)),
        }
    }

    fn before_call(&self, context: &FilesystemCallContext) -> Result<(), FileError> {
        if self.closed.get() {
            return Err(FileError::Io("filesystem provider is closed".into()));
        }
        context.check()
    }

    fn require_no_revision_expectation(
        mutation: &FilesystemMutationContext,
    ) -> Result<(), FileError> {
        if mutation.required() {
            Err(FileError::Unsupported)
        } else {
            Ok(())
        }
    }
}

impl<P: SynchronousFileProvider> IFilesystem for LegacyFilesystem<P> {
    fn descriptor(&self) -> FilesystemDescriptor {
        self.descriptor.clone()
    }

    fn stat<'a>(
        &'a self,
        context: FilesystemCallContext,
        path: String,
    ) -> FilesystemFuture<'a, FilesystemEntry> {
        Box::pin(async move {
            self.before_call(&context)?;
            self.provider.stat_entry(&path).map(FilesystemEntry::from)
        })
    }

    fn read<'a>(
        &'a self,
        context: FilesystemCallContext,
        path: String,
    ) -> FilesystemFuture<'a, Vec<u8>> {
        Box::pin(async move {
            self.before_call(&context)?;
            self.provider.read_bytes(&path)
        })
    }

    fn write<'a>(
        &'a self,
        context: FilesystemCallContext,
        path: String,
        bytes: Vec<u8>,
        options: WriteOptions,
        mutation: FilesystemMutationContext,
    ) -> FilesystemFuture<'a, FilesystemMutation> {
        Box::pin(async move {
            self.before_call(&context)?;
            Self::require_no_revision_expectation(&mutation)?;
            self.provider
                .write_bytes(&path, bytes, options)
                .map(FilesystemMutation::path)
        })
    }

    fn entries_page<'a>(
        &'a self,
        context: FilesystemCallContext,
        path: String,
        request: FilesystemPageRequest,
    ) -> FilesystemFuture<'a, FilesystemEntryPage> {
        Box::pin(async move {
            self.before_call(&context)?;
            if request.limit == 0 {
                return Err(FileError::InvalidPath(
                    "filesystem page limit must be positive".into(),
                ));
            }
            let start = request
                .token
                .as_deref()
                .unwrap_or("0")
                .parse::<usize>()
                .map_err(|_| FileError::InvalidPath("invalid filesystem page token".into()))?;
            let mut entries = self.provider.entries_values(&path)?;
            entries.sort_by(|left, right| left.path.cmp(&right.path));
            if start > entries.len() {
                return Err(FileError::InvalidPath(
                    "filesystem page token is outside the directory".into(),
                ));
            }
            let end = start.saturating_add(request.limit).min(entries.len());
            let next_token = (end < entries.len()).then(|| end.to_string());
            Ok(FilesystemEntryPage {
                entries: entries[start..end]
                    .iter()
                    .cloned()
                    .map(FilesystemEntry::from)
                    .collect(),
                next_token,
            })
        })
    }

    fn mkdir<'a>(
        &'a self,
        context: FilesystemCallContext,
        path: String,
        options: MkdirOptions,
        mutation: FilesystemMutationContext,
    ) -> FilesystemFuture<'a, FilesystemMutation> {
        Box::pin(async move {
            self.before_call(&context)?;
            Self::require_no_revision_expectation(&mutation)?;
            self.provider
                .mkdir_path(&path, options)
                .map(FilesystemMutation::path)
        })
    }

    fn delete<'a>(
        &'a self,
        context: FilesystemCallContext,
        path: String,
        options: DeleteOptions,
        mutation: FilesystemMutationContext,
    ) -> FilesystemFuture<'a, FilesystemMutation> {
        Box::pin(async move {
            self.before_call(&context)?;
            Self::require_no_revision_expectation(&mutation)?;
            self.provider
                .delete_path(&path, options)
                .map(FilesystemMutation::path)
        })
    }

    fn copy<'a>(
        &'a self,
        context: FilesystemCallContext,
        source: String,
        target: String,
        options: CopyOptions,
        mutation: FilesystemMutationContext,
    ) -> FilesystemFuture<'a, FilesystemMutation> {
        Box::pin(async move {
            self.before_call(&context)?;
            Self::require_no_revision_expectation(&mutation)?;
            self.provider
                .copy_path(&source, &target, options)
                .map(FilesystemMutation::path)
        })
    }

    fn move_entry<'a>(
        &'a self,
        context: FilesystemCallContext,
        source: String,
        target: String,
        options: MoveOptions,
        mutation: FilesystemMutationContext,
    ) -> FilesystemFuture<'a, FilesystemMutation> {
        Box::pin(async move {
            self.before_call(&context)?;
            Self::require_no_revision_expectation(&mutation)?;
            self.provider
                .move_path(&source, &target, options)
                .map(FilesystemMutation::path)
        })
    }

    fn close<'a>(&'a self, context: FilesystemCallContext) -> FilesystemFuture<'a, ()> {
        Box::pin(async move {
            context.check()?;
            self.closed.set(true);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_marks_the_legacy_adapter_closed() {
        let filesystem = LegacyFilesystem::new(
            crate::file::MemoryFileProvider::new("/"),
            FilesystemDescriptor::legacy("memory", "memory fixture"),
        );
        assert!(!filesystem.closed.get());
        filesystem.closed.set(true);
        assert!(matches!(
            filesystem.before_call(&FilesystemCallContext::default()),
            Err(FileError::Io(_))
        ));
    }
}
