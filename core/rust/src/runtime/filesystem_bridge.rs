use crate::core::Value;
use crate::file::{file_error_value, FileError, FileType};
use crate::filesystem::{
    FilesystemCallContext, FilesystemEntry, FilesystemFuture, FilesystemHandle,
    FilesystemPageRequest,
};
use crate::task::promise::WeakPromise;
use crate::task::{Promise, PromiseRejection, PromiseState};
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use std::time::{Duration, Instant};

pub(super) type ValueFuture = Pin<Box<dyn Future<Output = Result<Value, FileError>>>>;

pub(super) async fn collect_entries(
    handle: FilesystemHandle,
    context: FilesystemCallContext,
    path: String,
) -> Result<Vec<FilesystemEntry>, FileError> {
    let mut token = None;
    let mut entries = Vec::new();
    loop {
        context.check()?;
        let page = handle
            .as_filesystem()
            .entries_page(
                context.clone(),
                path.clone(),
                FilesystemPageRequest { token, limit: 256 },
            )
            .await?;
        entries.extend(page.entries);
        let Some(next) = page.next_token else {
            break;
        };
        token = Some(next);
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

pub(super) async fn walk_paths(
    handle: FilesystemHandle,
    context: FilesystemCallContext,
    path: String,
) -> Result<Vec<String>, FileError> {
    let root = handle
        .as_filesystem()
        .stat(context.clone(), path.clone())
        .await?;
    if root.kind != FileType::Directory {
        return Ok(vec![root.path]);
    }
    let mut directories = vec![path];
    let mut output = Vec::new();
    while let Some(directory) = directories.pop() {
        let mut children = collect_entries(handle.clone(), context.clone(), directory).await?;
        children.sort_by(|left, right| right.path.cmp(&left.path));
        for child in children {
            if child.kind == FileType::Directory {
                directories.push(child.path);
            } else {
                output.push(child.path);
            }
        }
    }
    output.sort();
    Ok(output)
}

pub(super) fn filesystem_entry_value(entry: FilesystemEntry) -> Value {
    let mut extensions = entry
        .extensions
        .into_iter()
        .map(|(key, value)| (Value::Keyword(key.into()), Value::String(value)))
        .collect::<Vec<_>>();
    if let Some(id) = entry.id {
        extensions.push((Value::Keyword("file/id".into()), Value::String(id)));
    }
    if let Some(revision) = entry.revision {
        extensions.push((
            Value::Keyword("file/revision".into()),
            Value::String(revision),
        ));
    }
    if let Some(capabilities) = entry.capabilities {
        extensions.push((
            Value::Keyword("provider/capabilities".into()),
            Value::Set(
                capabilities
                    .iter()
                    .map(|capability| Value::Keyword(capability.keyword().into()))
                    .collect(),
            ),
        ));
    }
    Value::Map(
        [
            (Value::Keyword("path".into()), Value::String(entry.path)),
            (Value::Keyword("name".into()), Value::String(entry.name)),
            (
                Value::Keyword("type".into()),
                Value::Keyword(entry.kind.keyword().into()),
            ),
            (
                Value::Keyword("size".into()),
                entry
                    .size
                    .and_then(|size| i64::try_from(size).ok())
                    .map(Value::Number)
                    .unwrap_or(Value::Nil),
            ),
            (
                Value::Keyword("modified-at".into()),
                entry.modified_at.map(Value::Number).unwrap_or(Value::Nil),
            ),
            (
                Value::Keyword("extensions".into()),
                Value::Map(extensions.into_iter().collect()),
            ),
        ]
        .into_iter()
        .collect(),
    )
}

struct FilesystemFutureDriver {
    future: RefCell<Option<ValueFuture>>,
    promise: WeakPromise,
    context: FilesystemCallContext,
    operation: String,
    path: String,
    target: Option<String>,
    polling: Cell<bool>,
}

impl FilesystemFutureDriver {
    fn poll_once(&self) {
        if self.polling.replace(true) {
            return;
        }
        let terminal = if self.context.cancelled() {
            Some(Err(stable_filesystem_error(
                "cancelled",
                &self.operation,
                &self.path,
                self.target.as_deref(),
            )))
        } else if self
            .context
            .deadline()
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            Some(Err(stable_filesystem_error(
                "timeout",
                &self.operation,
                &self.path,
                self.target.as_deref(),
            )))
        } else {
            let mut future = self.future.borrow_mut();
            future.as_mut().and_then(|future| {
                let waker = noop_waker();
                let mut task_context = Context::from_waker(&waker);
                match future.as_mut().poll(&mut task_context) {
                    Poll::Pending => None,
                    Poll::Ready(Ok(value)) => Some(Ok(value)),
                    Poll::Ready(Err(error)) => Some(Err(file_error_value(
                        &self.operation,
                        &self.path,
                        self.target.as_deref(),
                        &error,
                    ))),
                }
            })
        };
        self.polling.set(false);
        let Some(result) = terminal else {
            return;
        };
        self.future.borrow_mut().take();
        let Some(promise) = self.promise.upgrade() else {
            return;
        };
        match result {
            Ok(value) => {
                promise.resolve(value);
            }
            Err(error) => {
                promise.reject_value(error);
            }
        }
    }
}

pub(super) fn bridge_future(
    operation: &str,
    path: &str,
    target: Option<&str>,
    context: FilesystemCallContext,
    future: ValueFuture,
) -> Promise {
    let promise = Promise::new();
    let driver = Rc::new(FilesystemFutureDriver {
        future: RefCell::new(Some(future)),
        promise: promise.downgrade(),
        context: context.clone(),
        operation: operation.into(),
        path: path.into(),
        target: target.map(str::to_owned),
        polling: Cell::new(false),
    });
    let poller = driver.clone();
    promise.set_poller(Rc::new(move || poller.poll_once()));
    let waiter = driver.clone();
    let weak = promise.downgrade();
    promise.set_waiter(Rc::new(move || {
        #[cfg(target_arch = "wasm32")]
        waiter.poll_once();
        #[cfg(not(target_arch = "wasm32"))]
        while let Some(promise) = weak.upgrade() {
            if !matches!(promise.state(), PromiseState::Pending) {
                break;
            }
            waiter.poll_once();
            if matches!(promise.state(), PromiseState::Pending) {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }));
    let cancel_context = context;
    let cancel_weak = promise.downgrade();
    let cancel_operation = operation.to_owned();
    let cancel_path = path.to_owned();
    let cancel_target = target.map(str::to_owned);
    promise.set_cancel_hook(Rc::new(move || {
        cancel_context.cancel();
        if let Some(promise) = cancel_weak.upgrade() {
            promise.reject_rejection(PromiseRejection::Cancelled(stable_filesystem_error(
                "cancelled",
                &cancel_operation,
                &cancel_path,
                cancel_target.as_deref(),
            )));
        }
    }));
    promise
}

fn stable_filesystem_error(code: &str, operation: &str, path: &str, target: Option<&str>) -> Value {
    use crate::core::ExceptionInfo;
    Value::ExceptionInfo(Rc::new(ExceptionInfo {
        message: format!("{operation} failed: file/{code}"),
        data: Box::new(Value::Map(
            [
                (
                    Value::Keyword("ex/code".into()),
                    Value::Keyword(format!("file/{code}").into()),
                ),
                (
                    Value::Keyword("ex/class".into()),
                    Value::Keyword(
                        if code == "timeout" {
                            "ex.class/timeout"
                        } else {
                            "ex.class/io"
                        }
                        .into(),
                    ),
                ),
                (
                    Value::Keyword("file/operation".into()),
                    Value::Keyword(operation.trim_start_matches("file/").into()),
                ),
                (
                    Value::Keyword("file/path".into()),
                    Value::String(path.into()),
                ),
                (
                    Value::Keyword("file/target".into()),
                    target
                        .map(|value| Value::String(value.into()))
                        .unwrap_or(Value::Nil),
                ),
            ]
            .into_iter()
            .collect(),
        )),
        cause: None,
        provenance: Rc::new(RefCell::new(Default::default())),
    }))
}

fn noop_waker() -> Waker {
    unsafe fn clone(_: *const ()) -> RawWaker {
        raw_waker()
    }
    unsafe fn wake(_: *const ()) {}
    unsafe fn wake_by_ref(_: *const ()) {}
    unsafe fn drop(_: *const ()) {}
    fn raw_waker() -> RawWaker {
        RawWaker::new(
            std::ptr::null(),
            &RawWakerVTable::new(clone, wake, wake_by_ref, drop),
        )
    }
    unsafe { Waker::from_raw(raw_waker()) }
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn block_on_local<T>(mut future: FilesystemFuture<'_, T>) -> Result<T, FileError> {
    loop {
        let waker = noop_waker();
        let mut context = Context::from_waker(&waker);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::sleep(Duration::from_millis(1)),
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub(super) fn block_on_local<T>(_future: FilesystemFuture<'_, T>) -> Result<T, FileError> {
    Err(FileError::Unsupported)
}
