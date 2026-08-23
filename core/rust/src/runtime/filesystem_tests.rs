use super::*;
use crate::core::Value;
use crate::file::{
    CopyOptions, DeleteOptions, FileError, FileProvider, FileType, MkdirOptions, MoveOptions,
    WriteOptions,
};
use crate::filesystem::{
    FilesystemCallContext, FilesystemCapabilities, FilesystemCapability, FilesystemDescriptor,
    FilesystemEntry, FilesystemEntryPage, FilesystemFuture, FilesystemHandle, FilesystemMutation,
    FilesystemMutationContext, FilesystemPageRequest, IFilesystem,
};
use crate::task::PromiseState;
use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

struct OnePending<T> {
    pending: bool,
    value: Option<T>,
}

impl<T: Unpin> Future for OnePending<T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        if self.pending {
            self.pending = false;
            Poll::Pending
        } else {
            Poll::Ready(self.value.take().expect("future polled after completion"))
        }
    }
}

#[derive(Clone)]
struct FixtureFilesystem {
    closes: Rc<Cell<usize>>,
}

impl FixtureFilesystem {
    fn pending<T: Unpin + 'static>(value: Result<T, FileError>) -> FilesystemFuture<'static, T> {
        Box::pin(OnePending {
            pending: true,
            value: Some(value),
        })
    }
}

impl IFilesystem for FixtureFilesystem {
    fn descriptor(&self) -> FilesystemDescriptor {
        FilesystemDescriptor::new(
            "fixture",
            "redacted fixture",
            true,
            FilesystemCapabilities::new([
                FilesystemCapability::Read,
                FilesystemCapability::Entries,
            ]),
        )
    }

    fn stat<'a>(
        &'a self,
        _context: FilesystemCallContext,
        path: String,
    ) -> FilesystemFuture<'a, FilesystemEntry> {
        Self::pending(Ok(FilesystemEntry {
            name: path.rsplit('/').next().unwrap_or_default().into(),
            path,
            kind: FileType::File,
            size: Some(4),
            modified_at: None,
            id: Some("blob".into()),
            revision: Some("revision".into()),
            capabilities: None,
            extensions: Default::default(),
        }))
    }

    fn read<'a>(
        &'a self,
        _context: FilesystemCallContext,
        _path: String,
    ) -> FilesystemFuture<'a, Vec<u8>> {
        Self::pending(Ok(b"data".to_vec()))
    }

    fn write<'a>(
        &'a self,
        _context: FilesystemCallContext,
        _path: String,
        _bytes: Vec<u8>,
        _options: WriteOptions,
        _mutation: FilesystemMutationContext,
    ) -> FilesystemFuture<'a, FilesystemMutation> {
        Self::pending(Err(FileError::PermissionDenied))
    }

    fn entries_page<'a>(
        &'a self,
        _context: FilesystemCallContext,
        _path: String,
        _request: FilesystemPageRequest,
    ) -> FilesystemFuture<'a, FilesystemEntryPage> {
        Self::pending(Ok(FilesystemEntryPage {
            entries: Vec::new(),
            next_token: None,
        }))
    }

    fn mkdir<'a>(
        &'a self,
        _context: FilesystemCallContext,
        _path: String,
        _options: MkdirOptions,
        _mutation: FilesystemMutationContext,
    ) -> FilesystemFuture<'a, FilesystemMutation> {
        Self::pending(Err(FileError::PermissionDenied))
    }

    fn delete<'a>(
        &'a self,
        _context: FilesystemCallContext,
        _path: String,
        _options: DeleteOptions,
        _mutation: FilesystemMutationContext,
    ) -> FilesystemFuture<'a, FilesystemMutation> {
        Self::pending(Err(FileError::PermissionDenied))
    }

    fn copy<'a>(
        &'a self,
        _context: FilesystemCallContext,
        _source: String,
        _target: String,
        _options: CopyOptions,
        _mutation: FilesystemMutationContext,
    ) -> FilesystemFuture<'a, FilesystemMutation> {
        Self::pending(Err(FileError::PermissionDenied))
    }

    fn move_entry<'a>(
        &'a self,
        _context: FilesystemCallContext,
        _source: String,
        _target: String,
        _options: MoveOptions,
        _mutation: FilesystemMutationContext,
    ) -> FilesystemFuture<'a, FilesystemMutation> {
        Self::pending(Err(FileError::PermissionDenied))
    }

    fn close<'a>(&'a self, _context: FilesystemCallContext) -> FilesystemFuture<'a, ()> {
        self.closes.set(self.closes.get() + 1);
        Self::pending(Ok(()))
    }
}

#[test]
fn kernel_mount_routes_file_effects_through_ifilesystem() {
    let closes = Rc::new(Cell::new(0));
    let handle = FilesystemHandle::new(FixtureFilesystem {
        closes: closes.clone(),
    });
    let mut kernel = SessionKernel::new();
    let mount = kernel.create_provider_filesystem(handle);
    assert_eq!(
        kernel.filesystem_info(mount).unwrap(),
        ("provider", "redacted fixture", 0)
    );

    let child = SessionId::parse("provider-child").unwrap();
    kernel.create_session(child.clone()).unwrap();
    kernel.attach_filesystem(&child, mount).unwrap();
    let provider = kernel
        .session(&child)
        .unwrap()
        .runtime()
        .unwrap()
        .providers
        .file()
        .unwrap();
    let promise = provider.read("/data").unwrap();
    assert!(matches!(promise.state(), PromiseState::Pending));
    assert!(matches!(
        promise.state(),
        PromiseState::Fulfilled(Value::Bytes(bytes)) if bytes == b"data"
    ));
    assert!(kernel.close_filesystem(mount).is_err());

    drop(provider);
    kernel.detach_filesystem(&child).unwrap();
    kernel.close_filesystem(mount).unwrap();
    assert_eq!(closes.get(), 1);
}
