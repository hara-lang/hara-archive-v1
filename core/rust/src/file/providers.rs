//! Trusted remote filesystem provider projections.
//!
//! The runtime owns the logical filesystem contract.  Network clients are
//! deliberately supplied by the host through [`RemoteFilesystemClient`];
//! credentials, endpoint URLs, authentication, and transport state therefore
//! never become part of a mounted descriptor.

use crate::file::{
    CopyOptions, DeleteOptions, FileError, FileProvider, FileType, MkdirOptions, MoveOptions,
    WriteMode, WriteOptions,
};
use crate::filesystem::{
    FilesystemCallContext, FilesystemCapabilities, FilesystemCapability, FilesystemDescriptor,
    FilesystemEntry, FilesystemEntryPage, FilesystemFuture, FilesystemMutation,
    FilesystemMutationContext, FilesystemPageRequest, IFilesystem,
};
use std::cell::Cell;
use std::rc::Rc;

/// Provider kinds with a Rust semantic projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesystemProviderKind {
    Sftp,
    GoogleDrive,
    S3,
    GitHub,
    WebDav,
}

impl FilesystemProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sftp => "sftp",
            Self::GoogleDrive => "google-drive",
            Self::S3 => "s3",
            Self::GitHub => "github",
            Self::WebDav => "webdav",
        }
    }
}

/// Host-owned authenticated transport capability for a remote filesystem.
///
/// Implementations must keep credentials and transport details private.  The
/// methods return only provider-neutral values so callers cannot accidentally
/// expose access tokens, URLs, or provider-specific response objects.
pub trait RemoteFilesystemClient {
    fn authenticated(&self) -> bool;

    /// SFTP clients must override this with their host-key verification result.
    fn host_key_verified(&self) -> bool {
        true
    }

    /// HTTPS/WebDAV clients must override this when certificate or endpoint
    /// verification is not guaranteed by the host transport.
    fn transport_verified(&self) -> bool {
        true
    }

    fn capabilities(&self) -> FilesystemCapabilities;

    fn stat(&self, path: &str) -> Result<FilesystemEntry, FileError>;
    fn read(&self, path: &str) -> Result<Vec<u8>, FileError>;
    fn write(
        &self,
        path: &str,
        bytes: Vec<u8>,
        options: WriteOptions,
        mutation: &FilesystemMutationContext,
    ) -> Result<FilesystemMutation, FileError>;
    fn entries_page(
        &self,
        path: &str,
        request: &FilesystemPageRequest,
    ) -> Result<FilesystemEntryPage, FileError>;
    fn mkdir(
        &self,
        path: &str,
        options: MkdirOptions,
        mutation: &FilesystemMutationContext,
    ) -> Result<FilesystemMutation, FileError>;
    fn delete(
        &self,
        path: &str,
        options: DeleteOptions,
        mutation: &FilesystemMutationContext,
    ) -> Result<FilesystemMutation, FileError>;
    fn copy(
        &self,
        source: &str,
        target: &str,
        options: CopyOptions,
        mutation: &FilesystemMutationContext,
    ) -> Result<FilesystemMutation, FileError>;
    fn move_entry(
        &self,
        source: &str,
        target: &str,
        options: MoveOptions,
        mutation: &FilesystemMutationContext,
    ) -> Result<FilesystemMutation, FileError>;
    fn close(&self) -> Result<(), FileError>;
}

struct RemoteFilesystem {
    client: Rc<dyn RemoteFilesystemClient>,
    descriptor: FilesystemDescriptor,
    capabilities: FilesystemCapabilities,
    read_only: bool,
    closed: Rc<Cell<bool>>,
}

impl RemoteFilesystem {
    fn new(
        kind: FilesystemProviderKind,
        client: Rc<dyn RemoteFilesystemClient>,
        display: String,
        root: String,
        read_only: bool,
        blocked_capabilities: impl IntoIterator<Item = FilesystemCapability>,
        extra_extensions: impl IntoIterator<Item = (&'static str, String)>,
    ) -> Result<Self, FileError> {
        if !client.authenticated() {
            return Err(FileError::PermissionDenied);
        }
        let _root = crate::file::logical_normalise(&root)?;
        let capabilities =
            effective_capabilities(client.capabilities(), read_only, blocked_capabilities);
        let mut descriptor = FilesystemDescriptor::new(
            kind.as_str(),
            validate_display(display, kind.as_str())?,
            read_only,
            capabilities.clone(),
        )
        .with_extension("provider/root-scoped?", "true");
        for (key, value) in extra_extensions {
            descriptor = descriptor.with_extension(key, value);
        }
        Ok(Self {
            client,
            descriptor,
            capabilities,
            read_only,
            closed: Rc::new(Cell::new(false)),
        })
    }

    fn descriptor(&self) -> FilesystemDescriptor {
        self.descriptor.clone()
    }

    fn stat<'a>(
        &'a self,
        context: FilesystemCallContext,
        path: String,
    ) -> FilesystemFuture<'a, FilesystemEntry> {
        let path = match crate::file::logical_normalise(&path) {
            Ok(path) => path,
            Err(error) => return failed(error),
        };
        let client = self.client.clone();
        let closed = self.closed.clone();
        let capabilities = self.capabilities.clone();
        Box::pin(async move {
            context.check()?;
            ensure_open(&closed)?;
            ensure_capability(&capabilities, FilesystemCapability::Read)?;
            let entry = client.stat(&path)?;
            validate_entry(&entry, Some(&path))?;
            Ok(entry)
        })
    }

    fn read<'a>(
        &'a self,
        context: FilesystemCallContext,
        path: String,
    ) -> FilesystemFuture<'a, Vec<u8>> {
        let path = match crate::file::logical_normalise(&path) {
            Ok(path) => path,
            Err(error) => return failed(error),
        };
        let client = self.client.clone();
        let closed = self.closed.clone();
        let capabilities = self.capabilities.clone();
        Box::pin(async move {
            context.check()?;
            ensure_open(&closed)?;
            ensure_capability(&capabilities, FilesystemCapability::Read)?;
            client.read(&path)
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
        let path = match crate::file::logical_normalise(&path) {
            Ok(path) => path,
            Err(error) => return failed(error),
        };
        let client = self.client.clone();
        let closed = self.closed.clone();
        let capabilities = self.capabilities.clone();
        let read_only = self.read_only;
        Box::pin(async move {
            context.check()?;
            ensure_open(&closed)?;
            ensure_mutation(
                &capabilities,
                read_only,
                FilesystemCapability::Write,
                &mutation,
            )?;
            if options.mode == WriteMode::Append
                && !capabilities.contains(FilesystemCapability::Append)
            {
                return Err(FileError::Unsupported);
            }
            client.write(&path, bytes, options, &mutation)
        })
    }

    fn entries_page<'a>(
        &'a self,
        context: FilesystemCallContext,
        path: String,
        request: FilesystemPageRequest,
    ) -> FilesystemFuture<'a, FilesystemEntryPage> {
        let path = match crate::file::logical_normalise(&path) {
            Ok(path) => path,
            Err(error) => return failed(error),
        };
        let client = self.client.clone();
        let closed = self.closed.clone();
        let capabilities = self.capabilities.clone();
        Box::pin(async move {
            context.check()?;
            ensure_open(&closed)?;
            ensure_capability(&capabilities, FilesystemCapability::Entries)?;
            let page = client.entries_page(&path, &request)?;
            for entry in &page.entries {
                validate_entry(entry, None)?;
                if crate::file::logical_parent(&entry.path)?.as_deref() != Some(path.as_str()) {
                    return Err(FileError::InvalidPath(
                        "provider returned an entry outside its requested directory".into(),
                    ));
                }
            }
            if matches!(
                (&page.next_token, &request.token),
                (Some(next), Some(current)) if next == current
            ) {
                return Err(FileError::Io(
                    "provider returned a repeated page token".into(),
                ));
            }
            Ok(page)
        })
    }

    fn mkdir<'a>(
        &'a self,
        context: FilesystemCallContext,
        path: String,
        options: MkdirOptions,
        mutation: FilesystemMutationContext,
    ) -> FilesystemFuture<'a, FilesystemMutation> {
        self.mutation(
            context,
            path,
            mutation,
            FilesystemCapability::Mkdir,
            move |client, path, mutation| client.mkdir(&path, options, &mutation),
        )
    }

    fn delete<'a>(
        &'a self,
        context: FilesystemCallContext,
        path: String,
        options: DeleteOptions,
        mutation: FilesystemMutationContext,
    ) -> FilesystemFuture<'a, FilesystemMutation> {
        self.mutation(
            context,
            path,
            mutation,
            FilesystemCapability::Delete,
            move |client, path, mutation| client.delete(&path, options, &mutation),
        )
    }

    fn copy<'a>(
        &'a self,
        context: FilesystemCallContext,
        source: String,
        target: String,
        options: CopyOptions,
        mutation: FilesystemMutationContext,
    ) -> FilesystemFuture<'a, FilesystemMutation> {
        let source = match crate::file::logical_normalise(&source) {
            Ok(path) => path,
            Err(error) => return failed(error),
        };
        let target = match crate::file::logical_normalise(&target) {
            Ok(path) => path,
            Err(error) => return failed(error),
        };
        let client = self.client.clone();
        let closed = self.closed.clone();
        let capabilities = self.capabilities.clone();
        let read_only = self.read_only;
        Box::pin(async move {
            context.check()?;
            ensure_open(&closed)?;
            ensure_mutation(
                &capabilities,
                read_only,
                FilesystemCapability::Copy,
                &mutation,
            )?;
            if options.preserve_modified
                && !capabilities.contains(FilesystemCapability::PreserveModified)
            {
                return Err(FileError::Unsupported);
            }
            client.copy(&source, &target, options, &mutation)
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
        let source = match crate::file::logical_normalise(&source) {
            Ok(path) => path,
            Err(error) => return failed(error),
        };
        let target = match crate::file::logical_normalise(&target) {
            Ok(path) => path,
            Err(error) => return failed(error),
        };
        let client = self.client.clone();
        let closed = self.closed.clone();
        let capabilities = self.capabilities.clone();
        let read_only = self.read_only;
        Box::pin(async move {
            context.check()?;
            ensure_open(&closed)?;
            ensure_mutation(
                &capabilities,
                read_only,
                FilesystemCapability::Move,
                &mutation,
            )?;
            if options.atomic && !capabilities.contains(FilesystemCapability::AtomicMove) {
                return Err(FileError::Unsupported);
            }
            client.move_entry(&source, &target, options, &mutation)
        })
    }

    fn mutation<'a, F>(
        &'a self,
        context: FilesystemCallContext,
        path: String,
        mutation: FilesystemMutationContext,
        capability: FilesystemCapability,
        operation: F,
    ) -> FilesystemFuture<'a, FilesystemMutation>
    where
        F: FnOnce(
                Rc<dyn RemoteFilesystemClient>,
                String,
                FilesystemMutationContext,
            ) -> Result<FilesystemMutation, FileError>
            + 'a,
    {
        let path = match crate::file::logical_normalise(&path) {
            Ok(path) => path,
            Err(error) => return failed(error),
        };
        let client = self.client.clone();
        let closed = self.closed.clone();
        let capabilities = self.capabilities.clone();
        let read_only = self.read_only;
        Box::pin(async move {
            context.check()?;
            ensure_open(&closed)?;
            ensure_mutation(&capabilities, read_only, capability, &mutation)?;
            operation(client, path, mutation)
        })
    }

    fn close<'a>(&'a self, _context: FilesystemCallContext) -> FilesystemFuture<'a, ()> {
        let client = self.client.clone();
        let closed = self.closed.clone();
        Box::pin(async move {
            if closed.replace(true) {
                return Ok(());
            }
            client.close()
        })
    }
}

fn effective_capabilities(
    capabilities: FilesystemCapabilities,
    read_only: bool,
    blocked_capabilities: impl IntoIterator<Item = FilesystemCapability>,
) -> FilesystemCapabilities {
    let blocked_capabilities = blocked_capabilities
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    if read_only {
        FilesystemCapabilities::new(capabilities.iter().filter(|capability| {
            matches!(
                capability,
                FilesystemCapability::Read | FilesystemCapability::Entries
            ) && !blocked_capabilities.contains(capability)
        }))
    } else {
        FilesystemCapabilities::new(
            capabilities
                .iter()
                .filter(|capability| !blocked_capabilities.contains(capability)),
        )
    }
}

fn validate_display(display: String, kind: &str) -> Result<String, FileError> {
    if display.trim().is_empty() {
        return Err(FileError::InvalidPath(format!(
            "{kind} filesystem display must not be empty"
        )));
    }
    Ok(display)
}

fn ensure_open(closed: &Cell<bool>) -> Result<(), FileError> {
    if closed.get() {
        Err(FileError::Io("filesystem is closed".into()))
    } else {
        Ok(())
    }
}

fn ensure_capability(
    capabilities: &FilesystemCapabilities,
    capability: FilesystemCapability,
) -> Result<(), FileError> {
    capabilities
        .contains(capability)
        .then_some(())
        .ok_or(FileError::Unsupported)
}

fn ensure_mutation(
    capabilities: &FilesystemCapabilities,
    read_only: bool,
    capability: FilesystemCapability,
    mutation: &FilesystemMutationContext,
) -> Result<(), FileError> {
    if read_only {
        return Err(FileError::PermissionDenied);
    }
    ensure_capability(capabilities, capability)?;
    if mutation.required() && !capabilities.contains(FilesystemCapability::RevisionCheck) {
        return Err(FileError::Unsupported);
    }
    Ok(())
}

fn validate_entry(entry: &FilesystemEntry, expected_path: Option<&str>) -> Result<(), FileError> {
    let path = crate::file::logical_normalise(&entry.path)?;
    if path != entry.path {
        return Err(FileError::InvalidPath(
            "provider returned a non-canonical logical path".into(),
        ));
    }
    if let Some(expected_path) = expected_path {
        if path != expected_path {
            return Err(FileError::InvalidPath(
                "provider returned a path different from the requested entry".into(),
            ));
        }
    }
    if crate::file::logical_name(&path)? != entry.name {
        return Err(FileError::InvalidPath(
            "provider returned an entry with an invalid name".into(),
        ));
    }
    Ok(())
}

fn failed<'a, T>(error: FileError) -> FilesystemFuture<'a, T> {
    Box::pin(async move { Err(error) })
}

macro_rules! delegate_filesystem {
    ($provider:ty) => {
        impl IFilesystem for $provider {
            fn descriptor(&self) -> FilesystemDescriptor {
                self.core.descriptor()
            }

            fn stat<'a>(
                &'a self,
                context: FilesystemCallContext,
                path: String,
            ) -> FilesystemFuture<'a, FilesystemEntry> {
                self.core.stat(context, path)
            }

            fn read<'a>(
                &'a self,
                context: FilesystemCallContext,
                path: String,
            ) -> FilesystemFuture<'a, Vec<u8>> {
                self.core.read(context, path)
            }

            fn write<'a>(
                &'a self,
                context: FilesystemCallContext,
                path: String,
                bytes: Vec<u8>,
                options: WriteOptions,
                mutation: FilesystemMutationContext,
            ) -> FilesystemFuture<'a, FilesystemMutation> {
                self.core.write(context, path, bytes, options, mutation)
            }

            fn entries_page<'a>(
                &'a self,
                context: FilesystemCallContext,
                path: String,
                request: FilesystemPageRequest,
            ) -> FilesystemFuture<'a, FilesystemEntryPage> {
                self.core.entries_page(context, path, request)
            }

            fn mkdir<'a>(
                &'a self,
                context: FilesystemCallContext,
                path: String,
                options: MkdirOptions,
                mutation: FilesystemMutationContext,
            ) -> FilesystemFuture<'a, FilesystemMutation> {
                self.core.mkdir(context, path, options, mutation)
            }

            fn delete<'a>(
                &'a self,
                context: FilesystemCallContext,
                path: String,
                options: DeleteOptions,
                mutation: FilesystemMutationContext,
            ) -> FilesystemFuture<'a, FilesystemMutation> {
                self.core.delete(context, path, options, mutation)
            }

            fn copy<'a>(
                &'a self,
                context: FilesystemCallContext,
                source: String,
                target: String,
                options: CopyOptions,
                mutation: FilesystemMutationContext,
            ) -> FilesystemFuture<'a, FilesystemMutation> {
                self.core.copy(context, source, target, options, mutation)
            }

            fn move_entry<'a>(
                &'a self,
                context: FilesystemCallContext,
                source: String,
                target: String,
                options: MoveOptions,
                mutation: FilesystemMutationContext,
            ) -> FilesystemFuture<'a, FilesystemMutation> {
                self.core
                    .move_entry(context, source, target, options, mutation)
            }

            fn close<'a>(&'a self, context: FilesystemCallContext) -> FilesystemFuture<'a, ()> {
                self.core.close(context)
            }
        }
    };
}

/// A root-scoped, no-follow view over a host-owned SFTP capability.
///
/// SFTP paths are remote POSIX paths, so the generic remote projection cannot
/// be used directly: it would pass logical paths to the transport and would
/// not inspect ancestors before following them.  This adapter keeps the
/// configured root private while enforcing the same confinement rules as the
/// JVM provider.
struct ScopedSftpClient {
    client: Rc<dyn RemoteFilesystemClient>,
    root: String,
}

impl ScopedSftpClient {
    fn new(client: Rc<dyn RemoteFilesystemClient>, root: String) -> Self {
        Self { client, root }
    }

    fn remote_path(&self, path: &str) -> Result<String, FileError> {
        crate::file::logical_join(&self.root, path)
    }

    fn guard_ancestors(&self, path: &str) -> Result<(), FileError> {
        self.ensure_ancestors(path, false)
    }

    fn ensure_ancestors(&self, path: &str, parents: bool) -> Result<(), FileError> {
        let path = crate::file::logical_normalise(path)?;
        let segments = path
            .strip_prefix('/')
            .unwrap_or_default()
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        let mut current = "/".to_owned();
        for segment in segments.iter().take(segments.len().saturating_sub(1)) {
            current = crate::file::logical_join(&current, segment)?;
            let entry = match self.client.stat(&self.remote_path(&current)?) {
                Ok(entry) => entry,
                Err(FileError::NotFound) if parents => {
                    if !self
                        .client
                        .capabilities()
                        .contains(FilesystemCapability::Mkdir)
                    {
                        return Err(FileError::Unsupported);
                    }
                    self.client.mkdir(
                        &self.remote_path(&current)?,
                        MkdirOptions {
                            parents: false,
                            exists_ok: false,
                        },
                        &FilesystemMutationContext::default(),
                    )?;
                    self.client.stat(&self.remote_path(&current)?)?
                }
                Err(FileError::NotFound) => return Err(FileError::NotFound),
                Err(error) => return Err(error),
            };
            if entry.kind == FileType::Symlink {
                return Err(FileError::OutsideRoot);
            }
            if entry.kind != FileType::Directory {
                return Err(FileError::NotDirectory);
            }
        }
        Ok(())
    }

    fn rewrite_entry(
        &self,
        logical: &str,
        mut entry: FilesystemEntry,
    ) -> Result<FilesystemEntry, FileError> {
        let logical = crate::file::logical_normalise(logical)?;
        entry.path = logical.clone();
        entry.name = crate::file::logical_name(&logical)?;
        Ok(entry)
    }

    fn reject_symlink(entry: &FilesystemEntry) -> Result<(), FileError> {
        if entry.kind == FileType::Symlink {
            Err(FileError::Unsupported)
        } else {
            Ok(())
        }
    }

    fn stat_remote(&self, logical: &str) -> Result<FilesystemEntry, FileError> {
        self.guard_ancestors(logical)?;
        let remote = self.remote_path(logical)?;
        let entry = self.client.stat(&remote)?;
        self.rewrite_entry(logical, entry)
    }

    fn optional_stat(&self, logical: &str) -> Result<Option<FilesystemEntry>, FileError> {
        match self.client.stat(&self.remote_path(logical)?) {
            Ok(entry) => self.rewrite_entry(logical, entry).map(Some),
            Err(FileError::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

impl RemoteFilesystemClient for ScopedSftpClient {
    fn authenticated(&self) -> bool {
        self.client.authenticated()
    }

    fn host_key_verified(&self) -> bool {
        self.client.host_key_verified()
    }

    fn capabilities(&self) -> FilesystemCapabilities {
        self.client.capabilities()
    }

    fn stat(&self, path: &str) -> Result<FilesystemEntry, FileError> {
        self.stat_remote(path)
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, FileError> {
        let entry = self.stat_remote(path)?;
        Self::reject_symlink(&entry)?;
        if entry.kind == FileType::Directory {
            return Err(FileError::IsDirectory);
        }
        if entry.kind != FileType::File {
            return Err(FileError::Unsupported);
        }
        self.client.read(&self.remote_path(path)?)
    }

    fn write(
        &self,
        path: &str,
        bytes: Vec<u8>,
        options: WriteOptions,
        mutation: &FilesystemMutationContext,
    ) -> Result<FilesystemMutation, FileError> {
        self.ensure_ancestors(path, options.parents)?;
        if let Some(entry) = self.optional_stat(path)? {
            Self::reject_symlink(&entry)?;
            if entry.kind == FileType::Directory {
                return Err(FileError::IsDirectory);
            }
        }
        let mutation = self
            .client
            .write(&self.remote_path(path)?, bytes, options, mutation)?;
        Ok(FilesystemMutation {
            path: crate::file::logical_normalise(path)?,
            ..mutation
        })
    }

    fn entries_page(
        &self,
        path: &str,
        request: &FilesystemPageRequest,
    ) -> Result<FilesystemEntryPage, FileError> {
        let directory = self.stat_remote(path)?;
        if directory.kind == FileType::Symlink {
            return Err(FileError::Unsupported);
        }
        if directory.kind != FileType::Directory {
            return Err(FileError::NotDirectory);
        }
        let logical = crate::file::logical_normalise(path)?;
        let page = self
            .client
            .entries_page(&self.remote_path(&logical)?, request)?;
        let entries = page
            .entries
            .into_iter()
            .map(|entry| {
                let child = crate::file::logical_join(&logical, &entry.name)?;
                self.rewrite_entry(&child, entry)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(FilesystemEntryPage {
            entries,
            next_token: page.next_token,
        })
    }

    fn mkdir(
        &self,
        path: &str,
        options: MkdirOptions,
        mutation: &FilesystemMutationContext,
    ) -> Result<FilesystemMutation, FileError> {
        let logical = crate::file::logical_normalise(path)?;
        self.ensure_ancestors(&logical, options.parents)?;
        if let Some(entry) = self.optional_stat(&logical)? {
            if entry.kind == FileType::Directory && options.exists_ok {
                return Ok(FilesystemMutation::path(logical));
            }
            return Err(if entry.kind == FileType::Symlink {
                FileError::Unsupported
            } else {
                FileError::AlreadyExists
            });
        }
        let mutation = self
            .client
            .mkdir(&self.remote_path(&logical)?, options, mutation)?;
        Ok(FilesystemMutation {
            path: logical,
            ..mutation
        })
    }

    fn delete(
        &self,
        path: &str,
        options: DeleteOptions,
        mutation: &FilesystemMutationContext,
    ) -> Result<FilesystemMutation, FileError> {
        let logical = crate::file::logical_normalise(path)?;
        if logical == "/" {
            return Err(FileError::Denied);
        }
        self.guard_ancestors(&logical)?;
        let entry = match self.client.stat(&self.remote_path(&logical)?) {
            Ok(entry) => entry,
            Err(FileError::NotFound) if options.missing_ok => {
                return Ok(FilesystemMutation::path(logical));
            }
            Err(error) => return Err(error),
        };
        Self::reject_symlink(&entry)?;
        let mutation = self
            .client
            .delete(&self.remote_path(&logical)?, options, mutation)?;
        Ok(FilesystemMutation {
            path: logical,
            ..mutation
        })
    }

    fn copy(
        &self,
        source: &str,
        target: &str,
        options: CopyOptions,
        mutation: &FilesystemMutationContext,
    ) -> Result<FilesystemMutation, FileError> {
        let source = crate::file::logical_normalise(source)?;
        let target = crate::file::logical_normalise(target)?;
        if source == target {
            return Err(FileError::AlreadyExists);
        }
        self.guard_ancestors(&source)?;
        self.ensure_ancestors(&target, options.parents)?;
        let source_entry = self.stat_remote(&source)?;
        Self::reject_symlink(&source_entry)?;
        if source_entry.kind != FileType::File {
            return Err(FileError::Unsupported);
        }
        if let Some(target_entry) = self.optional_stat(&target)? {
            Self::reject_symlink(&target_entry)?;
            if target_entry.kind == FileType::Directory {
                return Err(FileError::IsDirectory);
            }
        }
        let mutation = self.client.copy(
            &self.remote_path(&source)?,
            &self.remote_path(&target)?,
            options,
            mutation,
        )?;
        Ok(FilesystemMutation {
            path: target,
            ..mutation
        })
    }

    fn move_entry(
        &self,
        source: &str,
        target: &str,
        options: MoveOptions,
        mutation: &FilesystemMutationContext,
    ) -> Result<FilesystemMutation, FileError> {
        let source = crate::file::logical_normalise(source)?;
        let target = crate::file::logical_normalise(target)?;
        if source == "/" || target == "/" {
            return Err(FileError::Denied);
        }
        if source == target {
            self.stat_remote(&source)?;
            return Ok(FilesystemMutation::path(target));
        }
        if target.starts_with(&(source.clone() + "/")) {
            return Err(FileError::InvalidPath(
                "cannot move an entry beneath itself".into(),
            ));
        }
        self.guard_ancestors(&source)?;
        self.ensure_ancestors(&target, options.parents)?;
        let source_entry = self.stat_remote(&source)?;
        Self::reject_symlink(&source_entry)?;
        if let Some(target_entry) = self.optional_stat(&target)? {
            Self::reject_symlink(&target_entry)?;
        }
        let mutation = self.client.move_entry(
            &self.remote_path(&source)?,
            &self.remote_path(&target)?,
            options,
            mutation,
        )?;
        Ok(FilesystemMutation {
            path: target,
            ..mutation
        })
    }

    fn close(&self) -> Result<(), FileError> {
        self.client.close()
    }
}

/// SFTP filesystem projection. Opening requires authentication and verified
/// host keys in addition to the normal provider capability checks.
pub struct SftpFilesystem {
    core: RemoteFilesystem,
}

impl SftpFilesystem {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn connect(
        options: crate::filesystem::sftp::SftpConnectOptions,
        root: impl Into<String>,
        display: impl Into<String>,
        read_only: bool,
    ) -> Result<Self, FileError> {
        let client = Rc::new(crate::filesystem::sftp::NativeSftpClient::connect(options)?);
        match Self::new(client.clone(), root, display, read_only) {
            Ok(filesystem) => Ok(filesystem),
            Err(error) => {
                // A root validation failure must tear down the authenticated
                // transport instead of leaving its worker alive in the
                // background.
                let _ = client.close();
                Err(error)
            }
        }
    }

    pub fn new(
        client: Rc<dyn RemoteFilesystemClient>,
        root: impl Into<String>,
        display: impl Into<String>,
        read_only: bool,
    ) -> Result<Self, FileError> {
        let root = sftp_root(root.into())?;
        if !client.host_key_verified() {
            return Err(FileError::PermissionDenied);
        }
        if !client.authenticated() {
            return Err(FileError::PermissionDenied);
        }
        let root_entry = client.stat(&root)?;
        if root_entry.kind == FileType::Symlink {
            return Err(FileError::OutsideRoot);
        }
        if root_entry.kind != FileType::Directory {
            return Err(FileError::NotDirectory);
        }
        let client = Rc::new(ScopedSftpClient::new(client, root));
        Ok(Self {
            core: RemoteFilesystem::new(
                FilesystemProviderKind::Sftp,
                client,
                display.into(),
                "/".into(),
                read_only,
                [],
                [("provider/host-key-verified?", "true".into())],
            )?,
        })
    }

    pub fn from_client<C: RemoteFilesystemClient + 'static>(
        client: C,
        root: impl Into<String>,
        display: impl Into<String>,
        read_only: bool,
    ) -> Result<Self, FileError> {
        Self::new(Rc::new(client), root, display, read_only)
    }
}

delegate_filesystem!(SftpFilesystem);

fn sftp_root(value: String) -> Result<String, FileError> {
    if value.trim().is_empty() || !value.starts_with('/') {
        return Err(FileError::InvalidPath(
            "SFTP root must be an absolute POSIX path".into(),
        ));
    }
    if value.contains('\0') || value.contains('\\') {
        return Err(FileError::InvalidPath(
            "SFTP root contains an invalid character".into(),
        ));
    }
    if value
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err(FileError::InvalidPath(
            "SFTP root cannot contain dot segments".into(),
        ));
    }
    crate::file::logical_normalise(&value)
}

/// Google Drive folder projection.  The root is a provider-owned stable ID,
/// not a user-visible URL or credential.
pub struct GoogleDriveFilesystem {
    core: RemoteFilesystem,
}

impl GoogleDriveFilesystem {
    pub fn new(
        client: Rc<dyn RemoteFilesystemClient>,
        root_id: impl Into<String>,
        display: impl Into<String>,
        read_only: bool,
    ) -> Result<Self, FileError> {
        let root_id = require_text(root_id.into(), "Google Drive root id")?;
        Ok(Self {
            core: RemoteFilesystem::new(
                FilesystemProviderKind::GoogleDrive,
                client,
                display.into(),
                "/".into(),
                read_only,
                [
                    FilesystemCapability::Append,
                    FilesystemCapability::AtomicMove,
                    FilesystemCapability::PreserveModified,
                ],
                [
                    ("provider/workspace-documents", "unsupported".into()),
                    ("provider/shared-drive?", "false".into()),
                    ("provider/root-id", root_id),
                ],
            )?,
        })
    }

    pub fn from_client<C: RemoteFilesystemClient + 'static>(
        client: C,
        root_id: impl Into<String>,
        display: impl Into<String>,
        read_only: bool,
    ) -> Result<Self, FileError> {
        Self::new(Rc::new(client), root_id, display, read_only)
    }
}

delegate_filesystem!(GoogleDriveFilesystem);

/// S3-compatible bucket/prefix projection with virtual-directory semantics
/// delegated to the authenticated client.
pub struct S3Filesystem {
    core: RemoteFilesystem,
}

impl S3Filesystem {
    pub fn new(
        client: Rc<dyn RemoteFilesystemClient>,
        bucket: impl Into<String>,
        prefix: impl Into<String>,
        display: impl Into<String>,
        read_only: bool,
    ) -> Result<Self, FileError> {
        let bucket = require_bucket(bucket.into())?;
        let prefix = validate_prefix(prefix.into())?;
        Ok(Self {
            core: RemoteFilesystem::new(
                FilesystemProviderKind::S3,
                client,
                display.into(),
                "/".into(),
                read_only,
                [
                    FilesystemCapability::Mkdir,
                    FilesystemCapability::Append,
                    FilesystemCapability::AtomicMove,
                    FilesystemCapability::PreserveModified,
                ],
                [
                    ("provider/virtual-directories?", "true".into()),
                    ("provider/atomic-move?", "false".into()),
                    ("provider/bucket", bucket),
                    ("provider/prefix", prefix),
                ],
            )?,
        })
    }

    pub fn from_client<C: RemoteFilesystemClient + 'static>(
        client: C,
        bucket: impl Into<String>,
        prefix: impl Into<String>,
        display: impl Into<String>,
        read_only: bool,
    ) -> Result<Self, FileError> {
        Self::new(Rc::new(client), bucket, prefix, display, read_only)
    }
}

delegate_filesystem!(S3Filesystem);

/// GitHub tree/blob projection.  Commit mode is writable only when the host
/// client advertises the required mutation capabilities.
pub struct GitHubFilesystem {
    core: RemoteFilesystem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitHubMountMode {
    ReadOnly,
    Commit,
}

impl GitHubFilesystem {
    pub fn new(
        client: Rc<dyn RemoteFilesystemClient>,
        repository: impl Into<String>,
        reference: impl Into<String>,
        root: impl Into<String>,
        mode: GitHubMountMode,
        display: impl Into<String>,
    ) -> Result<Self, FileError> {
        let repository = require_repository(repository.into())?;
        let reference = require_text(reference.into(), "GitHub ref")?;
        if matches!(mode, GitHubMountMode::Commit) && !reference.starts_with("heads/") {
            return Err(FileError::InvalidPath(
                "writable GitHub mounts require a heads/* ref".into(),
            ));
        }
        let read_only = matches!(mode, GitHubMountMode::ReadOnly);
        let root = crate::file::logical_normalise(&root.into())?;
        Ok(Self {
            core: RemoteFilesystem::new(
                FilesystemProviderKind::GitHub,
                client,
                display.into(),
                root.clone(),
                read_only,
                [
                    FilesystemCapability::Mkdir,
                    FilesystemCapability::Append,
                    FilesystemCapability::AtomicMove,
                    FilesystemCapability::PreserveModified,
                ],
                [
                    ("provider/repository", repository),
                    ("provider/ref", reference),
                    ("provider/root", root),
                    (
                        "provider/mode",
                        if read_only { "read-only" } else { "commit" }.into(),
                    ),
                ],
            )?,
        })
    }

    pub fn from_client<C: RemoteFilesystemClient + 'static>(
        client: C,
        repository: impl Into<String>,
        reference: impl Into<String>,
        root: impl Into<String>,
        mode: GitHubMountMode,
        display: impl Into<String>,
    ) -> Result<Self, FileError> {
        Self::new(Rc::new(client), repository, reference, root, mode, display)
    }
}

delegate_filesystem!(GitHubFilesystem);

/// WebDAV collection projection over a verified, authenticated client.
pub struct WebdavFilesystem {
    core: RemoteFilesystem,
}

impl WebdavFilesystem {
    pub fn new(
        client: Rc<dyn RemoteFilesystemClient>,
        root: impl Into<String>,
        display: impl Into<String>,
        read_only: bool,
    ) -> Result<Self, FileError> {
        if !client.transport_verified() {
            return Err(FileError::PermissionDenied);
        }
        Ok(Self {
            core: RemoteFilesystem::new(
                FilesystemProviderKind::WebDav,
                client,
                display.into(),
                root.into(),
                read_only,
                [
                    FilesystemCapability::Append,
                    FilesystemCapability::AtomicMove,
                    FilesystemCapability::PreserveModified,
                ],
                [("provider/transport-verified?", "true".into())],
            )?,
        })
    }

    pub fn from_client<C: RemoteFilesystemClient + 'static>(
        client: C,
        root: impl Into<String>,
        display: impl Into<String>,
        read_only: bool,
    ) -> Result<Self, FileError> {
        Self::new(Rc::new(client), root, display, read_only)
    }
}

delegate_filesystem!(WebdavFilesystem);

fn require_text(value: String, label: &str) -> Result<String, FileError> {
    if value.trim().is_empty() || value.contains('\0') {
        return Err(FileError::InvalidPath(format!("{label} must not be empty")));
    }
    Ok(value)
}

fn require_bucket(value: String) -> Result<String, FileError> {
    let value = require_text(value, "S3 bucket")?;
    if value.contains('/') || value.contains('\\') {
        return Err(FileError::InvalidPath(
            "S3 bucket must not contain path separators".into(),
        ));
    }
    Ok(value)
}

fn validate_prefix(value: String) -> Result<String, FileError> {
    if value.contains('\0') || value.contains('\\') {
        return Err(FileError::InvalidPath(
            "S3 prefix contains an invalid character".into(),
        ));
    }
    Ok(value.trim_matches('/').to_owned())
}

fn require_repository(value: String) -> Result<String, FileError> {
    let value = require_text(value, "GitHub repository")?;
    let mut parts = value.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(name), None)
            if !owner.is_empty()
                && !name.is_empty()
                && owner.chars().all(|character| {
                    character.is_ascii_alphanumeric() || "_.-".contains(character)
                })
                && name.chars().all(|character| {
                    character.is_ascii_alphanumeric() || "_.-".contains(character)
                }) =>
        {
            Ok(value)
        }
        _ => Err(FileError::InvalidPath(
            "GitHub repository must be owner/name".into(),
        )),
    }
}

/// A deterministic in-memory client useful for host integration tests and
/// browser adapters that have not yet bound a real transport.
#[derive(Clone)]
pub struct MemoryRemoteClient {
    provider: crate::file::MemoryFileProvider,
    capabilities: FilesystemCapabilities,
    authenticated: bool,
    host_key_verified: bool,
    closed: Rc<Cell<bool>>,
}

impl MemoryRemoteClient {
    pub fn new() -> Self {
        Self::with_capabilities(FilesystemCapabilities::legacy_read_write())
    }

    pub fn with_capabilities(capabilities: FilesystemCapabilities) -> Self {
        Self {
            provider: crate::file::MemoryFileProvider::new("/"),
            capabilities,
            authenticated: true,
            host_key_verified: true,
            closed: Rc::new(Cell::new(false)),
        }
    }

    pub fn unauthenticated(mut self) -> Self {
        self.authenticated = false;
        self
    }

    pub fn with_unverified_host_key(mut self) -> Self {
        self.host_key_verified = false;
        self
    }

    pub fn insert(&self, path: &str, bytes: Vec<u8>) -> Result<(), FileError> {
        self.provider.insert(path, bytes)
    }

    pub fn provider(&self) -> &crate::file::MemoryFileProvider {
        &self.provider
    }

    fn check_open(&self) -> Result<(), FileError> {
        if self.closed.get() {
            Err(FileError::Io("provider client is closed".into()))
        } else {
            Ok(())
        }
    }

    fn mutation(path: String) -> FilesystemMutation {
        FilesystemMutation::path(path)
    }
}

impl Default for MemoryRemoteClient {
    fn default() -> Self {
        Self::new()
    }
}

impl RemoteFilesystemClient for MemoryRemoteClient {
    fn authenticated(&self) -> bool {
        self.authenticated
    }

    fn host_key_verified(&self) -> bool {
        self.host_key_verified
    }

    fn capabilities(&self) -> FilesystemCapabilities {
        self.capabilities.clone()
    }

    fn stat(&self, path: &str) -> Result<FilesystemEntry, FileError> {
        self.check_open()?;
        Ok(self.provider.stat_entry(path)?.into())
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, FileError> {
        self.check_open()?;
        self.provider.read_bytes(path)
    }

    fn write(
        &self,
        path: &str,
        bytes: Vec<u8>,
        options: WriteOptions,
        _mutation: &FilesystemMutationContext,
    ) -> Result<FilesystemMutation, FileError> {
        self.check_open()?;
        Ok(Self::mutation(
            self.provider.write_bytes(path, bytes, options)?,
        ))
    }

    fn entries_page(
        &self,
        path: &str,
        request: &FilesystemPageRequest,
    ) -> Result<FilesystemEntryPage, FileError> {
        self.check_open()?;
        let entries = self
            .provider
            .entries_values(path)?
            .into_iter()
            .map(FilesystemEntry::from)
            .collect::<Vec<_>>();
        let offset = request
            .token
            .as_deref()
            .unwrap_or("0")
            .parse::<usize>()
            .map_err(|_| FileError::InvalidPath("invalid filesystem page token".into()))?;
        if offset > entries.len() {
            return Err(FileError::InvalidPath(
                "filesystem page token is out of range".into(),
            ));
        }
        let limit = request.limit.max(1);
        let end = offset.saturating_add(limit).min(entries.len());
        let next_token = (end < entries.len()).then(|| end.to_string());
        Ok(FilesystemEntryPage {
            entries: entries[offset..end].to_vec(),
            next_token,
        })
    }

    fn mkdir(
        &self,
        path: &str,
        options: MkdirOptions,
        _mutation: &FilesystemMutationContext,
    ) -> Result<FilesystemMutation, FileError> {
        self.check_open()?;
        Ok(Self::mutation(self.provider.mkdir_path(path, options)?))
    }

    fn delete(
        &self,
        path: &str,
        options: DeleteOptions,
        _mutation: &FilesystemMutationContext,
    ) -> Result<FilesystemMutation, FileError> {
        self.check_open()?;
        Ok(Self::mutation(self.provider.delete_path(path, options)?))
    }

    fn copy(
        &self,
        source: &str,
        target: &str,
        options: CopyOptions,
        _mutation: &FilesystemMutationContext,
    ) -> Result<FilesystemMutation, FileError> {
        self.check_open()?;
        Ok(Self::mutation(
            self.provider.copy_path(source, target, options)?,
        ))
    }

    fn move_entry(
        &self,
        source: &str,
        target: &str,
        options: MoveOptions,
        _mutation: &FilesystemMutationContext,
    ) -> Result<FilesystemMutation, FileError> {
        self.check_open()?;
        Ok(Self::mutation(
            self.provider.move_path(source, target, options)?,
        ))
    }

    fn close(&self) -> Result<(), FileError> {
        self.closed.set(true);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::{FilesystemCapability, FilesystemHandle};
    use crate::filesystem_bridge::block_on_local;

    #[test]
    fn all_remote_providers_publish_redacted_provider_descriptors() {
        let client = MemoryRemoteClient::new();
        client.insert("/hello.txt", b"hello".to_vec()).unwrap();

        let sftp = SftpFilesystem::from_client(
            client.clone().with_unverified_host_key(),
            "/srv",
            "trusted SFTP",
            false,
        );
        assert_eq!(sftp.err().unwrap().code(), "permission-denied");

        let drive = GoogleDriveFilesystem::from_client(client.clone(), "drive-root", "Drive", true)
            .unwrap();
        assert_eq!(drive.core.descriptor.kind(), "google-drive");
        assert_eq!(
            drive
                .core
                .descriptor
                .extensions()
                .get("provider/root-scoped?"),
            Some(&"true".to_string())
        );
        assert_eq!(
            drive.core.descriptor.extensions().get("provider/root-id"),
            Some(&"drive-root".to_string())
        );

        let s3 =
            S3Filesystem::from_client(client.clone(), "bucket", "prefix/", "S3", false).unwrap();
        assert_eq!(s3.core.descriptor.kind(), "s3");
        assert_eq!(
            s3.core.descriptor.extensions().get("provider/atomic-move?"),
            Some(&"false".to_string())
        );
        assert_eq!(
            s3.core.descriptor.extensions().get("provider/bucket"),
            Some(&"bucket".to_string())
        );
        assert_eq!(
            s3.core.descriptor.extensions().get("provider/prefix"),
            Some(&"prefix".to_string())
        );

        let github = GitHubFilesystem::from_client(
            client.clone(),
            "hara-lang/hara",
            "heads/main",
            "/",
            GitHubMountMode::Commit,
            "hara",
        )
        .unwrap();
        assert_eq!(
            github.core.descriptor.extensions().get("provider/mode"),
            Some(&"commit".to_string())
        );

        let webdav = WebdavFilesystem::from_client(client, "/remote", "WebDAV", true).unwrap();
        assert_eq!(webdav.core.descriptor.kind(), "webdav");
        assert_eq!(
            webdav
                .core
                .descriptor
                .extensions()
                .get("provider/transport-verified?"),
            Some(&"true".to_string())
        );
    }

    #[test]
    fn provider_operations_are_async_and_confined_to_canonical_paths() {
        let client = MemoryRemoteClient::new();
        client.insert("/hello.txt", b"hello".to_vec()).unwrap();
        let filesystem = SftpFilesystem::from_client(client, "/", "SFTP", false).unwrap();
        let entry = block_on_local(
            filesystem.stat(FilesystemCallContext::default(), "/./hello.txt".into()),
        )
        .unwrap();
        assert_eq!(entry.path, "/hello.txt");
        let bytes =
            block_on_local(filesystem.read(FilesystemCallContext::default(), "/hello.txt".into()))
                .unwrap();
        assert_eq!(bytes, b"hello");
        let page = block_on_local(filesystem.entries_page(
            FilesystemCallContext::default(),
            "/".into(),
            FilesystemPageRequest::default(),
        ))
        .unwrap();
        assert_eq!(page.entries.len(), 1);
        assert!(page.next_token.is_none());
        let error = block_on_local(
            filesystem.read(FilesystemCallContext::default(), "/../../secret".into()),
        )
        .unwrap_err();
        assert_eq!(error.code(), "outside-root");
    }

    #[test]
    fn sftp_mount_maps_logical_paths_to_the_private_remote_root() {
        let client = MemoryRemoteClient::new();
        client
            .insert("/srv/application/hello.txt", b"hello".to_vec())
            .unwrap();
        let filesystem =
            SftpFilesystem::from_client(client.clone(), "/srv/application", "SFTP", false).unwrap();

        let entry =
            block_on_local(filesystem.stat(FilesystemCallContext::default(), "/hello.txt".into()))
                .unwrap();
        assert_eq!(entry.path, "/hello.txt");
        assert_eq!(entry.name, "hello.txt");
        let bytes =
            block_on_local(filesystem.read(FilesystemCallContext::default(), "/hello.txt".into()))
                .unwrap();
        assert_eq!(bytes, b"hello");

        block_on_local(filesystem.write(
            FilesystemCallContext::default(),
            "/new.txt".into(),
            b"new".to_vec(),
            WriteOptions::default(),
            FilesystemMutationContext::default(),
        ))
        .unwrap();
        assert_eq!(
            client
                .provider()
                .read_bytes("/srv/application/new.txt")
                .unwrap(),
            b"new"
        );

        let invalid_root = SftpFilesystem::from_client(
            MemoryRemoteClient::new(),
            "/srv/../application",
            "SFTP",
            false,
        )
        .err()
        .unwrap();
        assert_eq!(invalid_root.code(), "invalid-path");
    }

    #[test]
    fn sftp_mutations_enforce_parent_and_mount_boundaries() {
        let client = MemoryRemoteClient::new();
        client.insert("/source.txt", b"source".to_vec()).unwrap();
        let filesystem = SftpFilesystem::from_client(client.clone(), "/", "SFTP", false).unwrap();

        let missing_parent = block_on_local(filesystem.write(
            FilesystemCallContext::default(),
            "/missing/file.txt".into(),
            b"data".to_vec(),
            WriteOptions::default(),
            FilesystemMutationContext::default(),
        ))
        .unwrap_err();
        assert_eq!(missing_parent.code(), "not-found");

        block_on_local(filesystem.write(
            FilesystemCallContext::default(),
            "/missing/file.txt".into(),
            b"data".to_vec(),
            WriteOptions {
                parents: true,
                ..WriteOptions::default()
            },
            FilesystemMutationContext::default(),
        ))
        .unwrap();
        assert_eq!(
            client.provider().read_bytes("/missing/file.txt").unwrap(),
            b"data"
        );

        let target_directory = block_on_local(filesystem.copy(
            FilesystemCallContext::default(),
            "/source.txt".into(),
            "/missing".into(),
            CopyOptions {
                replace: true,
                ..CopyOptions::default()
            },
            FilesystemMutationContext::default(),
        ))
        .unwrap_err();
        assert_eq!(target_directory.code(), "is-directory");

        let same_move = block_on_local(filesystem.move_entry(
            FilesystemCallContext::default(),
            "/source.txt".into(),
            "/./source.txt".into(),
            MoveOptions::default(),
            FilesystemMutationContext::default(),
        ))
        .unwrap();
        assert_eq!(same_move.path, "/source.txt");

        let root_delete = block_on_local(filesystem.delete(
            FilesystemCallContext::default(),
            "/".into(),
            DeleteOptions::default(),
            FilesystemMutationContext::default(),
        ))
        .unwrap_err();
        assert_eq!(root_delete.code(), "denied");
    }

    #[test]
    fn read_only_and_close_boundaries_are_deterministic() {
        let client = MemoryRemoteClient::new();
        let filesystem = WebdavFilesystem::from_client(client, "/", "WebDAV", true).unwrap();
        let write = block_on_local(filesystem.write(
            FilesystemCallContext::default(),
            "/new".into(),
            b"data".to_vec(),
            WriteOptions::default(),
            FilesystemMutationContext::default(),
        ))
        .unwrap_err();
        assert_eq!(write.code(), "permission-denied");
        block_on_local(filesystem.close(FilesystemCallContext::default())).unwrap();
        block_on_local(filesystem.close(FilesystemCallContext::default())).unwrap();
        let closed = block_on_local(filesystem.read(FilesystemCallContext::default(), "/".into()))
            .unwrap_err();
        assert_eq!(closed.code(), "io");
    }

    #[test]
    fn provider_handles_mount_through_the_runtime_adapter() {
        let client = MemoryRemoteClient::new();
        client.insert("/hello", b"world".to_vec()).unwrap();
        let handle = FilesystemHandle::new(
            WebdavFilesystem::from_client(client, "/", "WebDAV", true).unwrap(),
        );
        assert_eq!(handle.descriptor().kind(), "webdav");
        assert!(handle.capabilities().contains(FilesystemCapability::Read));
    }
}
