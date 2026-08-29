pub trait ExtensionProvider {
    fn name(&self) -> &str;
    fn install(&self, protocols: &mut ProtocolRegistry);
    fn construct(&self, type_name: &str, arguments: &[Value]) -> Result<Value, String>;
}

#[derive(Default, Clone)]
pub struct ExtensionRegistry {
    providers: HashMap<String, Rc<dyn ExtensionProvider>>,
    loaded: HashSet<String>,
}

impl ExtensionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn install<P: ExtensionProvider + 'static>(&mut self, provider: P) {
        self.providers
            .insert(provider.name().to_string(), Rc::new(provider));
    }

    pub fn contains(&self, name: &str) -> bool {
        self.providers.contains_key(name)
    }

    pub fn require(
        &mut self,
        name: &str,
        protocols: &mut ProtocolRegistry,
    ) -> Result<String, String> {
        let provider = self
            .providers
            .get(name)
            .cloned()
            .ok_or_else(|| format!("extension/not-found: {name}"))?;
        if self.loaded.insert(name.to_string()) {
            provider.install(protocols);
        }
        Ok(if self.loaded.len() == 1 {
            ":loaded".into()
        } else {
            ":loaded".into()
        })
    }

    pub fn construct(
        &self,
        provider: &str,
        type_name: &str,
        arguments: &[Value],
    ) -> Result<Value, String> {
        self.providers
            .get(provider)
            .ok_or_else(|| format!("extension/not-found: {provider}"))?
            .construct(type_name, arguments)
    }
}

#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub use crate::file::NativeFileProvider;
pub use crate::file::{
    CopyOptions, DeleteOptions, FileEntry, FileError, FileProvider, FileType, MemoryFileProvider,
    MkdirOptions, MoveOptions, TempDirectoryOptions, TempFileOptions, UnsupportedFileProvider,
    WriteMode, WriteOptions,
};

#[derive(Debug, Clone, PartialEq)]
pub enum SocketError {
    Unsupported,
    Denied,
    Invalid(String),
}

impl SocketError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::Denied => "denied",
            Self::Invalid(_) => "invalid",
        }
    }
}

pub type SocketHandle = u64;
pub type SocketCallback = Rc<dyn Fn(SocketEvent)>;

#[derive(Debug, Clone, PartialEq)]
pub enum SocketEvent {
    Connected(SocketHandle),
    Data(SocketHandle, Vec<u8>),
    Closed(SocketHandle),
    Failed(SocketHandle, String),
}

pub type SocketServerCallback = Rc<dyn Fn(SocketServerEvent)>;

#[derive(Debug, Clone, PartialEq)]
pub enum SocketServerEvent {
    Open {
        server: SocketHandle,
        connection: SocketHandle,
    },
    Data {
        server: SocketHandle,
        connection: SocketHandle,
        bytes: Vec<u8>,
    },
    Closed {
        server: SocketHandle,
        connection: SocketHandle,
    },
    Failed {
        server: SocketHandle,
        connection: SocketHandle,
        error: String,
    },
}

fn socket_server_event_value(event: SocketServerEvent) -> Value {
    let mut entries = Vec::new();
    match event {
        SocketServerEvent::Open { server, connection } => {
            entries.push((Value::Keyword("type".into()), Value::Keyword("open".into())));
            entries.push((
                Value::Keyword("server".into()),
                Value::Number(server as i64),
            ));
            entries.push((
                Value::Keyword("connection".into()),
                Value::Number(connection as i64),
            ));
        }
        SocketServerEvent::Data {
            server,
            connection,
            bytes,
        } => {
            entries.push((Value::Keyword("type".into()), Value::Keyword("data".into())));
            entries.push((
                Value::Keyword("server".into()),
                Value::Number(server as i64),
            ));
            entries.push((
                Value::Keyword("connection".into()),
                Value::Number(connection as i64),
            ));
            entries.push((Value::Keyword("bytes".into()), Value::Bytes(bytes)));
        }
        SocketServerEvent::Closed { server, connection } => {
            entries.push((
                Value::Keyword("type".into()),
                Value::Keyword("close".into()),
            ));
            entries.push((
                Value::Keyword("server".into()),
                Value::Number(server as i64),
            ));
            entries.push((
                Value::Keyword("connection".into()),
                Value::Number(connection as i64),
            ));
        }
        SocketServerEvent::Failed {
            server,
            connection,
            error,
        } => {
            entries.push((
                Value::Keyword("type".into()),
                Value::Keyword("error".into()),
            ));
            entries.push((
                Value::Keyword("server".into()),
                Value::Number(server as i64),
            ));
            entries.push((
                Value::Keyword("connection".into()),
                Value::Number(connection as i64),
            ));
            entries.push((Value::Keyword("error".into()), Value::String(error)));
        }
    }
    Value::Map(PMap::from_iter(entries))
}

pub trait SocketProvider {
    fn connect(
        &self,
        host: &str,
        port: u16,
        callback: SocketCallback,
    ) -> Result<SocketHandle, SocketError>;
    fn send(&self, socket: SocketHandle, bytes: &[u8]) -> Result<usize, SocketError>;
    fn close(&self, socket: SocketHandle) -> Result<(), SocketError>;
    fn listen(
        &self,
        _host: &str,
        _port: u16,
        _callback: SocketServerCallback,
    ) -> Result<SocketHandle, SocketError> {
        Err(SocketError::Unsupported)
    }
    fn endpoint(&self, _server: SocketHandle) -> Result<(String, u16), SocketError> {
        Err(SocketError::Unsupported)
    }
    fn events(&self, _handle: SocketHandle) -> Result<SocketHandle, SocketError> {
        Err(SocketError::Unsupported)
    }
    fn next(&self, _stream: SocketHandle) -> Result<Promise, SocketError> {
        Err(SocketError::Unsupported)
    }
}

#[cfg(not(target_arch = "wasm32"))]
use std::io::{Read, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::net::{Shutdown, TcpListener, TcpStream};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{mpsc, Arc, Mutex};

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
enum RawSocketEvent {
    Open {
        server: SocketHandle,
        connection: SocketHandle,
        stream: Arc<Mutex<TcpStream>>,
    },
    Data {
        server: SocketHandle,
        connection: SocketHandle,
        bytes: Vec<u8>,
    },
    Closed {
        server: SocketHandle,
        connection: SocketHandle,
    },
    Failed {
        server: SocketHandle,
        connection: SocketHandle,
        error: String,
    },
}

#[cfg(not(target_arch = "wasm32"))]
struct NativeServer {
    host: String,
    port: u16,
    alive: Arc<AtomicBool>,
}

#[cfg(not(target_arch = "wasm32"))]
struct NativeSocketStream {
    handle: SocketHandle,
    queue: VecDeque<Value>,
    queued_bytes: usize,
    pending: Option<Promise>,
    closed: bool,
}

#[cfg(not(target_arch = "wasm32"))]
struct NativeSocketState {
    next_handle: Arc<AtomicU64>,
    sockets: HashMap<SocketHandle, TcpStream>,
    callbacks: HashMap<SocketHandle, SocketCallback>,
    servers: HashMap<SocketHandle, NativeServer>,
    connections: HashMap<SocketHandle, Arc<Mutex<TcpStream>>>,
    connection_servers: HashMap<SocketHandle, SocketHandle>,
    server_callbacks: HashMap<SocketHandle, SocketServerCallback>,
    streams: HashMap<SocketHandle, NativeSocketStream>,
    sender: mpsc::Sender<RawSocketEvent>,
    receiver: mpsc::Receiver<RawSocketEvent>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct NativeSocketProvider {
    state: Rc<RefCell<NativeSocketState>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for NativeSocketProvider {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            state: Rc::new(RefCell::new(NativeSocketState {
                next_handle: Arc::new(AtomicU64::new(1)),
                sockets: HashMap::new(),
                callbacks: HashMap::new(),
                servers: HashMap::new(),
                connections: HashMap::new(),
                connection_servers: HashMap::new(),
                server_callbacks: HashMap::new(),
                streams: HashMap::new(),
                sender,
                receiver,
            })),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl NativeSocketProvider {
    fn next_handle(&self) -> SocketHandle {
        self.state
            .borrow()
            .next_handle
            .fetch_add(1, Ordering::Relaxed)
    }

    fn pump(&self) {
        loop {
            let event = { self.state.borrow().receiver.try_recv().ok() };
            let Some(event) = event else {
                break;
            };
            self.dispatch(event);
        }
    }

    fn wait_and_pump(&self) {
        let event = { self.state.borrow().receiver.recv().ok() };
        if let Some(event) = event {
            self.dispatch(event);
        }
        self.pump();
    }

    fn dispatch(&self, raw: RawSocketEvent) {
        let event = match raw {
            RawSocketEvent::Open {
                server,
                connection,
                stream,
            } => {
                let mut state = self.state.borrow_mut();
                state.connections.insert(connection, stream);
                state.connection_servers.insert(connection, server);
                SocketServerEvent::Open { server, connection }
            }
            RawSocketEvent::Data {
                server,
                connection,
                bytes,
            } => SocketServerEvent::Data {
                server,
                connection,
                bytes,
            },
            RawSocketEvent::Closed { server, connection } => {
                self.state.borrow_mut().connections.remove(&connection);
                SocketServerEvent::Closed { server, connection }
            }
            RawSocketEvent::Failed {
                server,
                connection,
                error,
            } => SocketServerEvent::Failed {
                server,
                connection,
                error,
            },
        };
        let callback = {
            self.state
                .borrow()
                .server_callbacks
                .get(&match &event {
                    SocketServerEvent::Open { server, .. }
                    | SocketServerEvent::Data { server, .. }
                    | SocketServerEvent::Closed { server, .. }
                    | SocketServerEvent::Failed { server, .. } => *server,
                })
                .cloned()
        };
        if let Some(callback) = callback {
            callback(event.clone());
        }
        let (server, connection, bytes) = match &event {
            SocketServerEvent::Open { server, connection }
            | SocketServerEvent::Closed { server, connection }
            | SocketServerEvent::Failed {
                server, connection, ..
            } => (*server, *connection, 0),
            SocketServerEvent::Data {
                server,
                connection,
                bytes,
            } => (*server, *connection, bytes.len()),
        };
        let value = socket_server_event_value(event);
        let overflow = {
            let mut state = self.state.borrow_mut();
            let mut overflow = false;
            for stream in state
                .streams
                .values_mut()
                .filter(|stream| stream.handle == server || stream.handle == connection)
            {
                if stream.closed {
                    continue;
                }
                if stream.queue.len() >= 256
                    || stream.queued_bytes.saturating_add(bytes) > 1_048_576
                {
                    stream.closed = true;
                    if let Some(promise) = stream.pending.take() {
                        promise.resolve(Value::Map(PMap::from_iter([
                            (
                                Value::Keyword("type".into()),
                                Value::Keyword("error".into()),
                            ),
                            (
                                Value::Keyword("error".into()),
                                Value::String("buffer-overflow".into()),
                            ),
                        ])));
                    }
                    overflow = true;
                    continue;
                }
                if let Some(promise) = stream.pending.take() {
                    promise.resolve(value.clone());
                } else {
                    stream.queued_bytes += bytes;
                    stream.queue.push_back(value.clone());
                }
            }
            overflow
        };
        if overflow {
            let _ = self.close(connection);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl SocketProvider for NativeSocketProvider {
    fn connect(
        &self,
        host: &str,
        port: u16,
        callback: SocketCallback,
    ) -> Result<SocketHandle, SocketError> {
        if host.is_empty() || port == 0 {
            return Err(SocketError::Invalid("host and port are required".into()));
        }
        let stream = TcpStream::connect((host, port))
            .map_err(|error| SocketError::Invalid(error.to_string()))?;
        let handle = self.next_handle();
        self.state.borrow_mut().sockets.insert(handle, stream);
        self.state
            .borrow_mut()
            .callbacks
            .insert(handle, callback.clone());
        callback(SocketEvent::Connected(handle));
        Ok(handle)
    }

    fn send(&self, socket: SocketHandle, bytes: &[u8]) -> Result<usize, SocketError> {
        let mut state = self.state.borrow_mut();
        if let Some(stream) = state.sockets.get_mut(&socket) {
            stream
                .write_all(bytes)
                .map_err(|error| SocketError::Invalid(error.to_string()))?;
            drop(state);
            if let Some(callback) = self.state.borrow().callbacks.get(&socket).cloned() {
                callback(SocketEvent::Data(socket, bytes.to_vec()));
            }
            return Ok(bytes.len());
        }
        let accepted = state.connections.get(&socket).cloned();
        drop(state);
        let accepted = accepted.ok_or_else(|| SocketError::Invalid("unknown socket".into()))?;
        accepted
            .lock()
            .map_err(|_| SocketError::Invalid("socket lock poisoned".into()))?
            .write_all(bytes)
            .map_err(|error| SocketError::Invalid(error.to_string()))?;
        Ok(bytes.len())
    }

    fn close(&self, socket: SocketHandle) -> Result<(), SocketError> {
        if self.state.borrow_mut().sockets.remove(&socket).is_some() {
            if let Some(callback) = self.state.borrow_mut().callbacks.remove(&socket) {
                callback(SocketEvent::Closed(socket));
            }
            return Ok(());
        }
        let server = { self.state.borrow_mut().servers.remove(&socket) };
        if let Some(server) = server {
            server.alive.store(false, Ordering::Relaxed);
            self.state.borrow_mut().server_callbacks.remove(&socket);
            return Ok(());
        }
        let (stream, server, sender) = {
            let mut state = self.state.borrow_mut();
            (
                state.connections.remove(&socket),
                state.connection_servers.remove(&socket).unwrap_or(0),
                state.sender.clone(),
            )
        };
        if let Some(stream) = stream {
            let _ = stream.lock().map(|stream| stream.shutdown(Shutdown::Both));
            let _ = sender.send(RawSocketEvent::Closed {
                server,
                connection: socket,
            });
            self.pump();
            return Ok(());
        }
        Err(SocketError::Invalid("unknown socket".into()))
    }

    fn listen(
        &self,
        host: &str,
        port: u16,
        callback: SocketServerCallback,
    ) -> Result<SocketHandle, SocketError> {
        if host.is_empty() {
            return Err(SocketError::Invalid("host is required".into()));
        }
        let listener = TcpListener::bind((host, port))
            .map_err(|error| SocketError::Invalid(error.to_string()))?;
        let endpoint = listener
            .local_addr()
            .map_err(|error| SocketError::Invalid(error.to_string()))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| SocketError::Invalid(error.to_string()))?;
        let server = self.next_handle();
        let alive = Arc::new(AtomicBool::new(true));
        let sender = self.state.borrow().sender.clone();
        let next_handle = self.state.borrow().next_handle.clone();
        let thread_alive = alive.clone();
        std::thread::Builder::new()
            .name(format!("hara-socket-{server}"))
            .spawn(move || {
                while thread_alive.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let connection = next_handle.fetch_add(1, Ordering::Relaxed);
                            if let Err(error) = stream.set_nonblocking(false) {
                                let _ = sender.send(RawSocketEvent::Failed {
                                    server,
                                    connection,
                                    error: error.to_string(),
                                });
                                continue;
                            }
                            let mut reader = match stream.try_clone() {
                                Ok(reader) => reader,
                                Err(error) => {
                                    let _ = sender.send(RawSocketEvent::Failed {
                                        server,
                                        connection,
                                        error: error.to_string(),
                                    });
                                    continue;
                                }
                            };
                            let shared = Arc::new(Mutex::new(stream));
                            let _ = sender.send(RawSocketEvent::Open {
                                server,
                                connection,
                                stream: shared.clone(),
                            });
                            let reader_sender = sender.clone();
                            std::thread::spawn(move || {
                                let mut buffer = [0u8; 8192];
                                loop {
                                    let read = reader.read(&mut buffer);
                                    match read {
                                        Ok(0) => {
                                            let _ = reader_sender.send(RawSocketEvent::Closed {
                                                server,
                                                connection,
                                            });
                                            break;
                                        }
                                        Ok(count) => {
                                            let _ = reader_sender.send(RawSocketEvent::Data {
                                                server,
                                                connection,
                                                bytes: buffer[..count].to_vec(),
                                            });
                                        }
                                        Err(error) => {
                                            let _ = reader_sender.send(RawSocketEvent::Failed {
                                                server,
                                                connection,
                                                error: error.to_string(),
                                            });
                                            break;
                                        }
                                    }
                                }
                            });
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(std::time::Duration::from_millis(5))
                        }
                        Err(error) => {
                            let _ = sender.send(RawSocketEvent::Failed {
                                server,
                                connection: 0,
                                error: error.to_string(),
                            });
                            break;
                        }
                    }
                }
            })
            .map_err(|error| SocketError::Invalid(error.to_string()))?;
        let mut state = self.state.borrow_mut();
        state.servers.insert(
            server,
            NativeServer {
                host: endpoint.ip().to_string(),
                port: endpoint.port(),
                alive,
            },
        );
        state.server_callbacks.insert(server, callback);
        Ok(server)
    }

    fn endpoint(&self, server: SocketHandle) -> Result<(String, u16), SocketError> {
        let state = self.state.borrow();
        let server = state
            .servers
            .get(&server)
            .ok_or_else(|| SocketError::Invalid("unknown socket server".into()))?;
        Ok((server.host.clone(), server.port))
    }

    fn events(&self, handle: SocketHandle) -> Result<SocketHandle, SocketError> {
        let mut state = self.state.borrow_mut();
        if !state.servers.contains_key(&handle) && !state.connections.contains_key(&handle) {
            return Err(SocketError::Invalid("unknown socket handle".into()));
        }
        let stream = state.next_handle.fetch_add(1, Ordering::Relaxed);
        state.streams.insert(
            stream,
            NativeSocketStream {
                handle,
                queue: VecDeque::new(),
                queued_bytes: 0,
                pending: None,
                closed: false,
            },
        );
        Ok(stream)
    }

    fn next(&self, stream: SocketHandle) -> Result<Promise, SocketError> {
        self.pump();
        let promise = Promise::new();
        {
            let mut state = self.state.borrow_mut();
            let stream = state
                .streams
                .get_mut(&stream)
                .ok_or_else(|| SocketError::Invalid("unknown socket stream".into()))?;
            if let Some(event) = stream.queue.pop_front() {
                stream.queued_bytes = 0;
                promise.resolve(event);
                return Ok(promise);
            }
            if stream.closed {
                promise.resolve(Value::Map(PMap::from_iter([(
                    Value::Keyword("type".into()),
                    Value::Keyword("close".into()),
                )])));
                return Ok(promise);
            }
            if stream.pending.is_some() {
                return Err(SocketError::Invalid(
                    "socket stream already has a pending next".into(),
                ));
            }
            stream.pending = Some(promise.clone());
        }
        let provider = self.clone();
        promise.set_poller(Rc::new(move || provider.pump()));
        let provider = self.clone();
        promise.set_waiter(Rc::new(move || provider.wait_and_pump()));
        Ok(promise)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub file: bool,
    pub socket: bool,
    pub process: bool,
}

pub struct ProviderRegistry {
    file: Option<Rc<dyn FileProvider>>,
    socket: Option<Rc<dyn SocketProvider>>,
    kernel: Option<Rc<KernelProvider>>,
    promise: Rc<dyn PromiseProvider>,
    process: bool,
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self {
            file: None,
            socket: None,
            kernel: None,
            promise: Rc::new(LocalPromiseProvider),
            process: false,
        }
    }
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn install_file<P: FileProvider + 'static>(&mut self, provider: P) {
        self.file = Some(Rc::new(provider));
    }
    pub fn set_file(&mut self, provider: Option<Rc<dyn FileProvider>>) {
        self.file = provider;
    }
    pub fn install_socket<P: SocketProvider + 'static>(&mut self, provider: P) {
        self.socket = Some(Rc::new(provider));
    }
    pub fn install_kernel(&mut self, provider: Rc<KernelProvider>) {
        self.kernel = Some(provider);
    }
    pub fn install_process(&mut self) {
        self.process = true;
    }
    pub fn install_promise<P: PromiseProvider + 'static>(&mut self, provider: P) {
        self.promise = Rc::new(provider);
    }
    pub fn promise(&self) -> Rc<dyn PromiseProvider> {
        self.promise.clone()
    }
    pub fn file(&self) -> Option<Rc<dyn FileProvider>> {
        self.file.clone()
    }
    pub fn socket(&self) -> Option<Rc<dyn SocketProvider>> {
        self.socket.clone()
    }
    pub fn kernel(&self) -> Option<Rc<KernelProvider>> {
        self.kernel.clone()
    }
    pub fn process(&self) -> bool {
        self.process
    }
    pub fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            file: self.file.is_some(),
            socket: self.socket.is_some(),
            process: self.process,
        }
    }
}

#[derive(Clone)]
pub struct LoopbackSocketProvider {
    next_handle: Rc<Cell<SocketHandle>>,
    callbacks: Rc<RefCell<HashMap<SocketHandle, SocketCallback>>>,
    streams: Rc<RefCell<HashMap<SocketHandle, LoopbackSocketStream>>>,
}

struct LoopbackSocketStream { socket: SocketHandle, queue: VecDeque<Value>, pending: Option<Promise>, closed: bool }

impl Default for LoopbackSocketProvider {
    fn default() -> Self {
        Self {
            next_handle: Rc::new(Cell::new(1)),
            callbacks: Rc::new(RefCell::new(HashMap::new())),
            streams: Rc::new(RefCell::new(HashMap::new())),
        }
    }
}

impl SocketProvider for LoopbackSocketProvider {
    fn connect(
        &self,
        host: &str,
        port: u16,
        callback: SocketCallback,
    ) -> Result<SocketHandle, SocketError> {
        if host.is_empty() || port == 0 {
            return Err(SocketError::Invalid("host and port are required".into()));
        }
        let handle = self.next_handle.get();
        self.next_handle.set(handle + 1);
        self.callbacks.borrow_mut().insert(handle, callback.clone());
        callback(SocketEvent::Connected(handle));
        Ok(handle)
    }

    fn send(&self, socket: SocketHandle, bytes: &[u8]) -> Result<usize, SocketError> {
        let callback = self
            .callbacks
            .borrow()
            .get(&socket)
            .cloned()
            .ok_or_else(|| SocketError::Invalid("unknown socket".into()))?;
        callback(SocketEvent::Data(socket, bytes.to_vec()));
        let event = socket_server_event_value(SocketServerEvent::Data { server: 0, connection: socket, bytes: bytes.to_vec() });
        for stream in self.streams.borrow_mut().values_mut().filter(|s| s.socket == socket) {
            if let Some(promise) = stream.pending.take() { promise.resolve(event.clone()); } else { stream.queue.push_back(event.clone()); }
        }
        Ok(bytes.len())
    }

    fn close(&self, socket: SocketHandle) -> Result<(), SocketError> {
        let callback = self
            .callbacks
            .borrow_mut()
            .remove(&socket)
            .ok_or_else(|| SocketError::Invalid("unknown socket".into()))?;
        callback(SocketEvent::Closed(socket));
        let event = socket_server_event_value(SocketServerEvent::Closed { server: 0, connection: socket });
        for stream in self.streams.borrow_mut().values_mut().filter(|s| s.socket == socket) {
            stream.closed = true;
            if let Some(promise) = stream.pending.take() { promise.resolve(event.clone()); } else { stream.queue.push_back(event.clone()); }
        }
        Ok(())
    }

    fn events(&self, socket: SocketHandle) -> Result<SocketHandle, SocketError> {
        if !self.callbacks.borrow().contains_key(&socket) { return Err(SocketError::Invalid("unknown socket".into())); }
        let handle = self.next_handle.get(); self.next_handle.set(handle + 1);
        self.streams.borrow_mut().insert(handle, LoopbackSocketStream { socket, queue: VecDeque::new(), pending: None, closed: false });
        Ok(handle)
    }

    fn next(&self, handle: SocketHandle) -> Result<Promise, SocketError> {
        let promise = Promise::new();
        let mut streams = self.streams.borrow_mut();
        let stream = streams.get_mut(&handle).ok_or_else(|| SocketError::Invalid("unknown socket stream".into()))?;
        if let Some(event) = stream.queue.pop_front() { promise.resolve(event); }
        else if stream.closed { promise.resolve(socket_server_event_value(SocketServerEvent::Closed { server: 0, connection: stream.socket })); }
        else if stream.pending.is_some() { return Err(SocketError::Invalid("socket stream already has a pending next".into())); }
        else { stream.pending = Some(promise.clone()); }
        Ok(promise)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UnsupportedSocketProvider;

impl SocketProvider for UnsupportedSocketProvider {
    fn connect(
        &self,
        _host: &str,
        _port: u16,
        _callback: SocketCallback,
    ) -> Result<SocketHandle, SocketError> {
        Err(SocketError::Unsupported)
    }
    fn send(&self, _socket: SocketHandle, _bytes: &[u8]) -> Result<usize, SocketError> {
        Err(SocketError::Unsupported)
    }
    fn close(&self, _socket: SocketHandle) -> Result<(), SocketError> {
        Err(SocketError::Unsupported)
    }
}

pub fn portable_type_name(value: &Value) -> &str {
    match value {
        Value::Nil => "nil",
        Value::Number(_) => "long",
        Value::Float(_) => "float",
        Value::BigInteger(_) if crate::numeric::is_long_value(value) => "long",
        Value::BigInteger(_) => "bigint",
        Value::Character(_) => "character",
        Value::Regex(_) => "pattern",
        Value::Tagged(_) => "tagged-literal",
        Value::Bool(_) => "boolean",
        Value::String(_) => "string",
        Value::Keyword(_) => "keyword",
        Value::Symbol(_) => "symbol",
        Value::Pointer(_) => "pointer",
        Value::Function(_) => "function",
        Value::Bytes(_) => "bytes",
        Value::ByteBuffer(_) => "byte-buffer",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::Promise(_) => "promise",
        Value::Atom(_) => "atom",
        Value::Recur(_) => "recur",
        Value::List(_) => "list",
        Value::Cons(_) => "cons",
        Value::Queue(_) => "queue",
        Value::Deque(_) => "deque",
        Value::Tuple(_) => "vector",
        Value::Vector(_) => "vector",
        Value::MapEntry(_) => "map-entry",
        Value::MutableCollection(_) => "mutable-collection",
        Value::Seq(_) => "seq",
        Value::Map(_) => "hash-map",
        Value::OrderedMap(_) => "ordered-map",
        Value::SortedMap(_) => "sorted-map",
        Value::Trie(_) => "trie",
        Value::PriorityMap(_) => "priority-map",
        Value::Set(_) => "hash-set",
        Value::OrderedSet(_) => "ordered-set",
        Value::SortedSet(_) => "sorted-set",
        Value::Iterator(_) => "iterator",
        Value::Var(_) => "var",
        Value::Namespace(_) => "namespace",
        Value::Extension(_) => "extension",
        Value::StructType(_) => "struct-type",
        Value::Struct(_) => "struct",
        Value::MutableType(_) => "mutable-type",
        Value::Mutable(_) => "mutable",
        Value::Protocol(_) => "protocol",
        Value::NativeType(_) => "native-type",
        Value::Schema(_) => "schema",
        Value::Coroutine(_) => "coroutine",
        Value::Stream(_) => "stream",
        Value::Result(_) => "result",
        Value::ExceptionInfo(_) => "exception",
    }
}
