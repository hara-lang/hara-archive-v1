use super::filesystem_bridge::{
    block_on_local, bridge_future, collect_entries, filesystem_entry_value, walk_paths, ValueFuture,
};
use crate::core::Value;
use crate::file::{
    logical_normalise, logical_resolve, CopyOptions, DeleteOptions, FileEntry, FileError,
    FileProvider, MkdirOptions, MoveOptions, TempDirectoryOptions, TempFileOptions, WriteMode,
    WriteOptions,
};
use crate::filesystem::{FilesystemCallContext, FilesystemHandle, FilesystemMutationContext};
use crate::task::Promise;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

static PROVIDER_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);
const TEMP_ATTEMPTS: usize = 128;

/// Routes the existing `std.native.File` promise API through an asynchronous
/// provider-neutral [`FilesystemHandle`]. Synchronous primitive methods remain
/// unavailable so remote and browser providers are never weakened to blocking
/// result semantics.
#[derive(Clone)]
pub struct FilesystemRuntimeAdapter {
    state: Rc<FilesystemRuntimeState>,
}

struct FilesystemRuntimeState {
    handle: FilesystemHandle,
    close_started: Cell<bool>,
}

impl FilesystemRuntimeAdapter {
    pub fn new(handle: FilesystemHandle) -> Self {
        Self {
            state: Rc::new(FilesystemRuntimeState {
                handle,
                close_started: Cell::new(false),
            }),
        }
    }

    pub fn handle(&self) -> FilesystemHandle {
        self.state.handle.clone()
    }

    pub fn close(&self) -> Promise {
        if self.state.close_started.replace(true) {
            let promise = Promise::new();
            promise.resolve(Value::Nil);
            return promise;
        }
        let context = FilesystemCallContext::default();
        let handle = self.handle();
        let future_context = context.clone();
        bridge_future(
            "file/close",
            "/",
            None,
            context,
            Box::pin(async move {
                handle
                    .as_filesystem()
                    .close(future_context)
                    .await
                    .map(|()| Value::Nil)
            }),
        )
    }

    fn effect(
        &self,
        operation: &'static str,
        path: String,
        target: Option<String>,
        future: ValueFuture,
        context: FilesystemCallContext,
    ) -> Promise {
        bridge_future(operation, &path, target.as_deref(), context, future)
    }
}

impl Drop for FilesystemRuntimeState {
    fn drop(&mut self) {
        if self.close_started.replace(true) {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let context = FilesystemCallContext::default();
            let future = self.handle.as_filesystem().close(context);
            let _ = block_on_local(future);
        }
    }
}

impl FileProvider for FilesystemRuntimeAdapter {
    fn read_bytes(&self, _path: &str) -> Result<Vec<u8>, FileError> {
        Err(FileError::Unsupported)
    }

    fn write_bytes(
        &self,
        _path: &str,
        _bytes: Vec<u8>,
        _options: WriteOptions,
    ) -> Result<String, FileError> {
        Err(FileError::Unsupported)
    }

    fn exists_value(&self, _path: &str) -> Result<bool, FileError> {
        Err(FileError::Unsupported)
    }

    fn stat_entry(&self, _path: &str) -> Result<FileEntry, FileError> {
        Err(FileError::Unsupported)
    }

    fn entries_values(&self, _path: &str) -> Result<Vec<FileEntry>, FileError> {
        Err(FileError::Unsupported)
    }

    fn mkdir_path(&self, _path: &str, _options: MkdirOptions) -> Result<String, FileError> {
        Err(FileError::Unsupported)
    }

    fn delete_path(&self, _path: &str, _options: DeleteOptions) -> Result<String, FileError> {
        Err(FileError::Unsupported)
    }

    fn copy_path(
        &self,
        _source: &str,
        _target: &str,
        _options: CopyOptions,
    ) -> Result<String, FileError> {
        Err(FileError::Unsupported)
    }

    fn move_path(
        &self,
        _source: &str,
        _target: &str,
        _options: MoveOptions,
    ) -> Result<String, FileError> {
        Err(FileError::Unsupported)
    }

    fn temp_file_path(
        &self,
        _parent: &str,
        _options: TempFileOptions,
    ) -> Result<String, FileError> {
        Err(FileError::Unsupported)
    }

    fn temp_directory_path(
        &self,
        _parent: &str,
        _options: TempDirectoryOptions,
    ) -> Result<String, FileError> {
        Err(FileError::Unsupported)
    }

    fn read(&self, path: &str) -> Result<Promise, FileError> {
        let path = logical_normalise(path)?;
        let context = FilesystemCallContext::default();
        let future_context = context.clone();
        let handle = self.handle();
        let future_path = path.clone();
        let future = Box::pin(async move {
            handle
                .as_filesystem()
                .read(future_context, future_path)
                .await
                .map(Value::Bytes)
        });
        Ok(self.effect("file/read", path, None, future, context))
    }

    fn write(&self, path: &str, bytes: Vec<u8>) -> Result<Promise, FileError> {
        self.write_with_options(path, bytes, WriteOptions::default())
    }

    fn write_with_options(
        &self,
        path: &str,
        bytes: Vec<u8>,
        options: WriteOptions,
    ) -> Result<Promise, FileError> {
        let path = logical_normalise(path)?;
        let context = FilesystemCallContext::default();
        let future_context = context.clone();
        let handle = self.handle();
        let future_path = path.clone();
        let future = Box::pin(async move {
            handle
                .as_filesystem()
                .write(
                    future_context,
                    future_path,
                    bytes,
                    options,
                    FilesystemMutationContext::default(),
                )
                .await
                .map(|mutation| Value::String(mutation.path))
        });
        Ok(self.effect("file/write", path, None, future, context))
    }

    fn exists(&self, path: &str) -> Result<Promise, FileError> {
        let path = logical_normalise(path)?;
        let context = FilesystemCallContext::default();
        let future_context = context.clone();
        let handle = self.handle();
        let future_path = path.clone();
        let future = Box::pin(async move {
            match handle
                .as_filesystem()
                .stat(future_context, future_path)
                .await
            {
                Ok(_) => Ok(Value::Bool(true)),
                Err(FileError::NotFound) => Ok(Value::Bool(false)),
                Err(error) => Err(error),
            }
        });
        Ok(self.effect("file/exists?", path, None, future, context))
    }

    fn stat(&self, path: &str) -> Result<Promise, FileError> {
        let path = logical_normalise(path)?;
        let context = FilesystemCallContext::default();
        let future_context = context.clone();
        let handle = self.handle();
        let future_path = path.clone();
        let future = Box::pin(async move {
            handle
                .as_filesystem()
                .stat(future_context, future_path)
                .await
                .map(filesystem_entry_value)
        });
        Ok(self.effect("file/stat", path, None, future, context))
    }

    fn entries(&self, path: &str) -> Result<Promise, FileError> {
        let path = logical_normalise(path)?;
        let context = FilesystemCallContext::default();
        let handle = self.handle();
        let future_context = context.clone();
        let future_path = path.clone();
        let future = Box::pin(async move {
            collect_entries(handle, future_context, future_path)
                .await
                .map(|entries| {
                    Value::Vector(entries.into_iter().map(filesystem_entry_value).collect())
                })
        });
        Ok(self.effect("file/entries", path, None, future, context))
    }

    fn list(&self, path: &str) -> Result<Promise, FileError> {
        let path = logical_normalise(path)?;
        let context = FilesystemCallContext::default();
        let handle = self.handle();
        let future_context = context.clone();
        let future_path = path.clone();
        let future = Box::pin(async move {
            collect_entries(handle, future_context, future_path)
                .await
                .map(|entries| {
                    Value::Vector(
                        entries
                            .into_iter()
                            .map(|entry| Value::String(entry.path))
                            .collect(),
                    )
                })
        });
        Ok(self.effect("file/list", path, None, future, context))
    }

    fn walk(&self, path: &str) -> Result<Promise, FileError> {
        let path = logical_normalise(path)?;
        let context = FilesystemCallContext::default();
        let handle = self.handle();
        let future_context = context.clone();
        let future_path = path.clone();
        let future = Box::pin(async move {
            walk_paths(handle, future_context, future_path)
                .await
                .map(|paths| Value::Vector(paths.into_iter().map(Value::String).collect()))
        });
        Ok(self.effect("file/walk", path, None, future, context))
    }

    fn mkdir(&self, path: &str) -> Result<Promise, FileError> {
        self.mkdir_with_options(path, MkdirOptions::default())
    }

    fn mkdir_with_options(&self, path: &str, options: MkdirOptions) -> Result<Promise, FileError> {
        let path = logical_normalise(path)?;
        let context = FilesystemCallContext::default();
        let future_context = context.clone();
        let handle = self.handle();
        let future_path = path.clone();
        let future = Box::pin(async move {
            handle
                .as_filesystem()
                .mkdir(
                    future_context,
                    future_path,
                    options,
                    FilesystemMutationContext::default(),
                )
                .await
                .map(|mutation| Value::String(mutation.path))
        });
        Ok(self.effect("file/mkdir", path, None, future, context))
    }

    fn delete(&self, path: &str) -> Result<Promise, FileError> {
        self.delete_with_options(path, DeleteOptions::default())
    }

    fn delete_with_options(
        &self,
        path: &str,
        options: DeleteOptions,
    ) -> Result<Promise, FileError> {
        let path = logical_normalise(path)?;
        let context = FilesystemCallContext::default();
        let future_context = context.clone();
        let handle = self.handle();
        let future_path = path.clone();
        let future = Box::pin(async move {
            handle
                .as_filesystem()
                .delete(
                    future_context,
                    future_path,
                    options,
                    FilesystemMutationContext::default(),
                )
                .await
                .map(|mutation| Value::String(mutation.path))
        });
        Ok(self.effect("file/delete", path, None, future, context))
    }

    fn copy(&self, source: &str, target: &str, options: CopyOptions) -> Result<Promise, FileError> {
        let source = logical_normalise(source)?;
        let target = logical_normalise(target)?;
        let context = FilesystemCallContext::default();
        let future_context = context.clone();
        let handle = self.handle();
        let future_source = source.clone();
        let future_target = target.clone();
        let future = Box::pin(async move {
            handle
                .as_filesystem()
                .copy(
                    future_context,
                    future_source,
                    future_target,
                    options,
                    FilesystemMutationContext::default(),
                )
                .await
                .map(|mutation| Value::String(mutation.path))
        });
        Ok(self.effect("file/copy", source, Some(target), future, context))
    }

    fn move_entry(
        &self,
        source: &str,
        target: &str,
        options: MoveOptions,
    ) -> Result<Promise, FileError> {
        let source = logical_normalise(source)?;
        let target = logical_normalise(target)?;
        let context = FilesystemCallContext::default();
        let future_context = context.clone();
        let handle = self.handle();
        let future_source = source.clone();
        let future_target = target.clone();
        let future = Box::pin(async move {
            handle
                .as_filesystem()
                .move_entry(
                    future_context,
                    future_source,
                    future_target,
                    options,
                    FilesystemMutationContext::default(),
                )
                .await
                .map(|mutation| Value::String(mutation.path))
        });
        Ok(self.effect("file/move", source, Some(target), future, context))
    }

    fn temp_file(&self, parent: &str, options: TempFileOptions) -> Result<Promise, FileError> {
        let parent = logical_normalise(parent)?;
        let context = FilesystemCallContext::default();
        let future_context = context.clone();
        let handle = self.handle();
        let future_parent = parent.clone();
        let future = Box::pin(async move {
            for _ in 0..TEMP_ATTEMPTS {
                future_context.check()?;
                let sequence = PROVIDER_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let path = logical_resolve(
                    &future_parent,
                    &format!("{}-{sequence}{}", options.prefix, options.suffix),
                )?;
                match handle
                    .as_filesystem()
                    .write(
                        future_context.clone(),
                        path.clone(),
                        Vec::new(),
                        WriteOptions {
                            mode: WriteMode::Create,
                            parents: false,
                        },
                        FilesystemMutationContext::default(),
                    )
                    .await
                {
                    Ok(_) => return Ok(Value::String(path)),
                    Err(FileError::AlreadyExists) => continue,
                    Err(error) => return Err(error),
                }
            }
            Err(FileError::Io(
                "temporary file attempts were exhausted".into(),
            ))
        });
        Ok(self.effect("file/temp-file", parent, None, future, context))
    }

    fn temp_directory(
        &self,
        parent: &str,
        options: TempDirectoryOptions,
    ) -> Result<Promise, FileError> {
        let parent = logical_normalise(parent)?;
        let context = FilesystemCallContext::default();
        let future_context = context.clone();
        let handle = self.handle();
        let future_parent = parent.clone();
        let future = Box::pin(async move {
            for _ in 0..TEMP_ATTEMPTS {
                future_context.check()?;
                let sequence = PROVIDER_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let path =
                    logical_resolve(&future_parent, &format!("{}-{sequence}", options.prefix))?;
                match handle
                    .as_filesystem()
                    .mkdir(
                        future_context.clone(),
                        path.clone(),
                        MkdirOptions {
                            parents: false,
                            exists_ok: false,
                        },
                        FilesystemMutationContext::default(),
                    )
                    .await
                {
                    Ok(_) => return Ok(Value::String(path)),
                    Err(FileError::AlreadyExists) => continue,
                    Err(error) => return Err(error),
                }
            }
            Err(FileError::Io(
                "temporary directory attempts were exhausted".into(),
            ))
        });
        Ok(self.effect("file/temp-directory", parent, None, future, context))
    }
}
