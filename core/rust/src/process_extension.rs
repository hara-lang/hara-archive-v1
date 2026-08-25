#![cfg(not(target_arch = "wasm32"))]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::rc::{Rc, Weak};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::core::{Promise, Value};
use crate::extension::{ExtensionManifest, WasmAbi, WasmExtensionProvider};
use crate::hta;

const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

type SharedWriter = Arc<Mutex<BufWriter<ChildStdin>>>;

enum ReaderEvent {
    Frame(Vec<u8>),
    Closed(String),
}

struct PendingRequest {
    promise: Promise,
    deadline: Option<Instant>,
}

struct Dispatcher {
    receiver: RefCell<mpsc::Receiver<ReaderEvent>>,
    pending: RefCell<HashMap<u64, PendingRequest>>,
    terminal: RefCell<Option<String>>,
}

impl Dispatcher {
    fn new(receiver: mpsc::Receiver<ReaderEvent>) -> Self {
        Self {
            receiver: RefCell::new(receiver),
            pending: RefCell::new(HashMap::new()),
            terminal: RefCell::new(None),
        }
    }

    fn insert(&self, request: u64, promise: Promise, timeout: Option<Duration>) {
        let deadline = timeout.map(|duration| Instant::now() + duration);
        self.pending
            .borrow_mut()
            .insert(request, PendingRequest { promise, deadline });
    }

    fn remove(&self, request: u64) {
        self.pending.borrow_mut().remove(&request);
    }

    fn poll(&self) {
        loop {
            match self.receiver.borrow().try_recv() {
                Ok(event) => self.dispatch(event),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.fail_all("hta/process-closed: response reader stopped".into());
                    break;
                }
            }
        }
        self.expire();
    }

    fn wait(&self, request: u64) {
        self.poll();
        while self.pending.borrow().contains_key(&request) {
            let wait = self
                .pending
                .borrow()
                .get(&request)
                .and_then(|pending| pending.deadline)
                .map_or(Duration::from_secs(3600), |deadline| {
                    deadline.saturating_duration_since(Instant::now())
                });
            if wait.is_zero() {
                self.timeout(request);
                break;
            }
            let event = self.receiver.borrow().recv_timeout(wait);
            match event {
                Ok(event) => self.dispatch(event),
                Err(mpsc::RecvTimeoutError::Timeout) => self.timeout(request),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.fail_all("hta/process-closed: response reader stopped".into());
                }
            }
        }
    }

    fn dispatch(&self, event: ReaderEvent) {
        match event {
            ReaderEvent::Frame(bytes) => match hta::decode(&bytes).and_then(Self::response) {
                Ok((request, result)) => {
                    if let Some(pending) = self.pending.borrow_mut().remove(&request) {
                        match result {
                            Ok(value) => {
                                pending.promise.resolve(value);
                            }
                            Err(error) => {
                                pending.promise.reject(error);
                            }
                        }
                    }
                }
                Err(error) => self.fail_all(error),
            },
            ReaderEvent::Closed(error) => self.fail_all(error),
        }
    }

    fn response(frame: Value) -> Result<(u64, Result<Value, String>), String> {
        let values = match frame {
            Value::Vector(values) => values.iter().cloned().collect::<Vec<_>>(),
            Value::List(values) => values.iter().cloned().collect::<Vec<_>>(),
            _ => return Err("hta/process-frame-malformed".into()),
        };
        if values.len() < 3 {
            return Err("hta/process-frame-malformed".into());
        }
        let request = match values[1] {
            Value::Number(request) if request >= 0 => request as u64,
            _ => return Err("hta/process-frame-malformed".into()),
        };
        let result = match &values[0] {
            Value::String(kind) if kind == "result" => Ok(values[2].clone()),
            Value::String(kind) if kind == "error" => {
                Err(format!("hta/remote-error: {}", values[2].display()))
            }
            _ => return Err("hta/process-event-unknown".into()),
        };
        Ok((request, result))
    }

    fn expire(&self) {
        let now = Instant::now();
        let expired = self
            .pending
            .borrow()
            .iter()
            .filter_map(|(request, pending)| {
                pending
                    .deadline
                    .filter(|deadline| *deadline <= now)
                    .map(|_| *request)
            })
            .collect::<Vec<_>>();
        for request in expired {
            self.timeout(request);
        }
    }

    fn timeout(&self, request: u64) {
        if let Some(pending) = self.pending.borrow_mut().remove(&request) {
            pending.promise.notify_cancel();
            pending.promise.reject("hta/process-timeout");
        }
    }

    fn fail_all(&self, error: String) {
        if self.terminal.borrow().is_some() {
            return;
        }
        *self.terminal.borrow_mut() = Some(error.clone());
        for (_, pending) in self.pending.borrow_mut().drain() {
            pending.promise.reject(error.clone());
        }
    }
}

struct Session {
    child: Child,
    output: SharedWriter,
    dispatcher: Rc<Dispatcher>,
    reader: Option<JoinHandle<()>>,
    next_request: Cell<u64>,
    timeout: Option<Duration>,
}

pub struct ProcessExtensionProvider {
    module: std::path::PathBuf,
    session: RefCell<Option<Session>>,
}

impl ProcessExtensionProvider {
    pub fn new(module: std::path::PathBuf) -> Self {
        Self {
            module,
            session: RefCell::new(None),
        }
    }

    fn write(output: &SharedWriter, value: &Value) -> Result<(), String> {
        let bytes = hta::encode(value)?;
        if bytes.len() > MAX_FRAME_BYTES {
            return Err("hta/frame-too-large".into());
        }
        let mut output = output
            .lock()
            .map_err(|_| "hta/process-write-failed: writer lock poisoned".to_owned())?;
        output
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .and_then(|_| output.write_all(&bytes))
            .and_then(|_| output.flush())
            .map_err(|error| format!("hta/process-write-failed: {error}"))
    }

    fn read(input: &mut BufReader<ChildStdout>) -> Result<Vec<u8>, String> {
        let mut header = [0_u8; 4];
        input
            .read_exact(&mut header)
            .map_err(|error| format!("hta/process-closed: {error}"))?;
        let length = u32::from_be_bytes(header) as usize;
        if length == 0 || length > MAX_FRAME_BYTES {
            return Err(format!("hta/process-frame-size: {length}"));
        }
        let mut bytes = vec![0; length];
        input
            .read_exact(&mut bytes)
            .map_err(|error| format!("hta/process-closed: {error}"))?;
        Ok(bytes)
    }

    fn list(values: impl IntoIterator<Item = Value>) -> Value {
        Value::List(values.into_iter().collect())
    }

    fn timeout() -> Option<Duration> {
        match std::env::var("HARA_HTA_TIMEOUT_MS") {
            Ok(value) => match value.parse::<u64>() {
                Ok(0) => None,
                Ok(milliseconds) => Some(Duration::from_millis(milliseconds)),
                Err(_) => Some(DEFAULT_TIMEOUT),
            },
            Err(_) => Some(DEFAULT_TIMEOUT),
        }
    }

    fn install_hooks(
        dispatcher: &Rc<Dispatcher>,
        output: &SharedWriter,
        request: u64,
        promise: &Promise,
    ) {
        let weak: Weak<Dispatcher> = Rc::downgrade(dispatcher);
        promise.set_poller(Rc::new(move || {
            if let Some(dispatcher) = weak.upgrade() {
                dispatcher.poll();
            }
        }));
        let weak: Weak<Dispatcher> = Rc::downgrade(dispatcher);
        promise.set_waiter(Rc::new(move || {
            if let Some(dispatcher) = weak.upgrade() {
                dispatcher.wait(request);
            }
        }));
        let weak: Weak<Dispatcher> = Rc::downgrade(dispatcher);
        let output = output.clone();
        promise.set_cancel_hook(Rc::new(move || {
            if let Some(dispatcher) = weak.upgrade() {
                dispatcher.remove(request);
                let _ = Self::write(
                    &output,
                    &Self::list([
                        Value::String("cancel".into()),
                        Value::Number(request as i64),
                    ]),
                );
            }
        }));
    }
}

impl WasmExtensionProvider for ProcessExtensionProvider {
    fn supports(&self, abi: WasmAbi) -> bool {
        abi == WasmAbi::HtaV1
    }

    fn capabilities(&self) -> Vec<String> {
        vec!["process".into()]
    }

    fn start(&self, manifest: &ExtensionManifest) -> Result<(), String> {
        if manifest
            .capabilities
            .iter()
            .any(|capability| capability != "process")
        {
            return Err("extension/capability-invalid: unsupported managed Node capability".into());
        }
        let node = std::env::var("HARA_NODE").unwrap_or_else(|_| "node".into());
        let directory = self.module.parent().unwrap_or_else(|| Path::new("."));
        let mut child = Command::new(node)
            .arg(&self.module)
            .current_dir(directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| {
                format!("hta/process-start-failed: {} ({error})", manifest.namespace)
            })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "hta/process-start-failed: missing stdout".to_owned())?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "hta/process-start-failed: missing stdin".to_owned())?;
        let output = Arc::new(Mutex::new(BufWriter::new(stdin)));
        Self::write(
            &output,
            &Self::list([
                Value::String("handshake".into()),
                Value::Number(1),
                Value::String(manifest.namespace.clone()),
                Value::Vector(
                    manifest
                        .exports
                        .iter()
                        .map(|(name, _)| Value::String(name.clone()))
                        .collect::<Vec<_>>()
                        .into(),
                ),
            ]),
        )?;
        let mut input = BufReader::new(stdout);
        let ready = hta::decode(&Self::read(&mut input)?)?;
        let values = match &ready {
            Value::List(values) => values.iter().collect::<Vec<_>>(),
            Value::Vector(values) => values.iter().collect(),
            _ => Vec::new(),
        };
        if values.len() < 2
            || values[0] != &Value::String("ready".into())
            || values[1] != &Value::Number(1)
        {
            let _ = child.kill();
            return Err(format!("hta/handshake-invalid: {}", ready.display()));
        }

        let (sender, receiver) = mpsc::channel();
        let reader = std::thread::Builder::new()
            .name(format!("hara-hta-{}", manifest.namespace))
            .spawn(move || loop {
                match Self::read(&mut input) {
                    Ok(bytes) => {
                        if sender.send(ReaderEvent::Frame(bytes)).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(ReaderEvent::Closed(error));
                        break;
                    }
                }
            })
            .map_err(|error| format!("hta/process-start-failed: reader ({error})"))?;
        *self.session.borrow_mut() = Some(Session {
            child,
            output,
            dispatcher: Rc::new(Dispatcher::new(receiver)),
            reader: Some(reader),
            next_request: Cell::new(0),
            timeout: Self::timeout(),
        });
        Ok(())
    }

    fn invoke(
        &self,
        manifest: &ExtensionManifest,
        export: &str,
        arguments: &[Value],
    ) -> Result<Value, String> {
        let session = self.session.borrow();
        let session = session
            .as_ref()
            .ok_or_else(|| format!("hta/process-unavailable: {}", manifest.namespace))?;
        let request = session.next_request.get() + 1;
        session.next_request.set(request);
        let promise = Promise::new();
        Self::install_hooks(&session.dispatcher, &session.output, request, &promise);
        session
            .dispatcher
            .insert(request, promise.clone(), session.timeout);
        if let Err(error) = Self::write(
            &session.output,
            &Self::list([
                Value::String("call".into()),
                Value::Number(request as i64),
                Value::String(export.into()),
                Value::Vector(arguments.to_vec().into()),
            ]),
        ) {
            session.dispatcher.remove(request);
            return Err(error);
        }
        Ok(Value::Promise(promise))
    }

    fn cancel(&self, _manifest: &ExtensionManifest, request: u64) -> Result<(), String> {
        let session = self.session.borrow();
        let session = session
            .as_ref()
            .ok_or_else(|| "hta/process-unavailable".to_owned())?;
        session.dispatcher.remove(request);
        Self::write(
            &session.output,
            &Self::list([
                Value::String("cancel".into()),
                Value::Number(request as i64),
            ]),
        )
    }

    fn shutdown(&self, _manifest: &ExtensionManifest) {
        if let Some(mut session) = self.session.borrow_mut().take() {
            session.dispatcher.fail_all("hta/process-shutdown".into());
            let _ = Self::write(
                &session.output,
                &Self::list([Value::String("shutdown".into())]),
            );
            let _ = session.child.kill();
            let _ = session.child.wait();
            if let Some(reader) = session.reader.take() {
                let _ = reader.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Dispatcher, PendingRequest, ProcessExtensionProvider, ReaderEvent};
    use crate::core::{Promise, PromiseState, Value};
    use crate::extension::{ExtensionManifest, WasmExtensionProvider};
    use crate::task::PromiseRejection;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    const MANIFEST: &str = r#"
{:namespace "fixture.process"
 :version "1"
 :provider :hta
 :abi :hta.v1
 :targets {:node {:provider "worker.mjs" :runtime :process}}
 :exports {"later" {:args [:integer :integer] :returns :integer :async true}
           "crash" {:args [] :returns :value :async true}}
 :capabilities [:process]}"#;

    fn result(promise: Promise) -> Result<Value, String> {
        match promise.wait_state() {
            PromiseState::Fulfilled(value) => Ok(value),
            PromiseState::Rejected(error) => Err(error.message()),
            PromiseState::Pending => Err("still pending".into()),
        }
    }

    fn promise(value: Value) -> Promise {
        match value {
            Value::Promise(promise) => promise,
            _ => panic!("expected promise"),
        }
    }

    #[test]
    fn dispatcher_times_out_and_propagates_reader_failure() {
        let (_sender, receiver) = mpsc::channel();
        let dispatcher = Dispatcher::new(receiver);
        let timed_out = Promise::new();
        dispatcher.pending.borrow_mut().insert(
            1,
            PendingRequest {
                promise: timed_out.clone(),
                deadline: Some(std::time::Instant::now()),
            },
        );
        dispatcher.poll();
        assert_eq!(
            timed_out.state(),
            PromiseState::Rejected("hta/process-timeout".into())
        );

        let pending = Promise::new();
        dispatcher.pending.borrow_mut().insert(
            2,
            PendingRequest {
                promise: pending.clone(),
                deadline: None,
            },
        );
        dispatcher.dispatch(ReaderEvent::Closed("hta/process-closed: boom".into()));
        assert_eq!(
            pending.state(),
            PromiseState::Rejected("hta/process-closed: boom".into())
        );
    }

    #[test]
    fn node_process_handles_out_of_order_cancel_and_crash() {
        if std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("skipping fake Node provider test: node is unavailable");
            return;
        }
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let module =
            std::env::temp_dir().join(format!("hara-hta-fake-{}-{id}.mjs", std::process::id()));
        let hta = std::path::Path::new(env!("HARA_SOURCE_ROOT"))
            .join("web/packages/hta/index.js")
            .canonicalize()
            .unwrap();
        let source = format!(
            r#"import {{decodeHta,encodeHta}} from "file://{}";
let buffered=new Uint8Array(), expected=null;
const timers=new Map();
process.stdin.on("data",chunk=>{{const next=new Uint8Array(buffered.length+chunk.length);next.set(buffered);next.set(chunk,buffered.length);buffered=next;drain();}});
function drain(){{for(;;){{if(expected===null){{if(buffered.length<4)return;expected=new DataView(buffered.buffer,buffered.byteOffset,4).getUint32(0,false);buffered=buffered.slice(4);}}if(buffered.length<expected)return;const frame=buffered.slice(0,expected);buffered=buffered.slice(expected);expected=null;dispatch(decodeHta(frame));}}}}
function dispatch(frame){{const [kind,id,operation,args]=frame;if(kind==="handshake")return write(["ready",1]);if(kind==="shutdown")return process.exit(0);if(kind==="cancel"){{clearTimeout(timers.get(Number(id)));timers.delete(Number(id));return;}}if(operation==="crash")return process.exit(7);const request=Number(id),timer=setTimeout(()=>{{timers.delete(request);write(["result",request,args[1]]);}},Number(args[0]));timers.set(request,timer);}}
function write(value){{const frame=encodeHta(value),header=new Uint8Array(4);new DataView(header.buffer).setUint32(0,frame.length,false);process.stdout.write(header);process.stdout.write(frame);}}
"#,
            hta.display()
        );
        std::fs::write(&module, source).unwrap();
        let manifest = ExtensionManifest::parse(MANIFEST, "fixture").unwrap();
        let provider = ProcessExtensionProvider::new(module.clone());
        provider.start(&manifest).unwrap();

        let slow = promise(
            provider
                .invoke(&manifest, "later", &[Value::Number(40), Value::Number(1)])
                .unwrap(),
        );
        let fast = promise(
            provider
                .invoke(&manifest, "later", &[Value::Number(1), Value::Number(2)])
                .unwrap(),
        );
        assert_eq!(result(fast).unwrap(), Value::Number(2));
        assert_eq!(result(slow).unwrap(), Value::Number(1));

        let cancelled = promise(
            provider
                .invoke(&manifest, "later", &[Value::Number(100), Value::Number(3)])
                .unwrap(),
        );
        assert!(cancelled.cancel());
        assert!(matches!(
            cancelled.state(),
            PromiseState::Rejected(PromiseRejection::Cancelled(value))
                if matches!(value, Value::Map(_))
        ));

        let crashed = promise(provider.invoke(&manifest, "crash", &[]).unwrap());
        assert!(result(crashed)
            .unwrap_err()
            .starts_with("hta/process-closed:"));
        provider.shutdown(&manifest);
        let _ = std::fs::remove_file(module);

        // Leave enough time for a mistakenly uncancelled fake request to expose itself.
        std::thread::sleep(Duration::from_millis(5));
    }
}
