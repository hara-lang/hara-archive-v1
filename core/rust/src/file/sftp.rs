//! Native SSH/SFTP transport for the provider-neutral filesystem surface.
//!
//! The public provider remains synchronous at the host capability seam, while
//! the SSH/SFTP protocol runs on one private Tokio worker. Credentials and
//! host-key policy stay in this transport and never enter a filesystem
//! descriptor.

use super::providers::RemoteFilesystemClient;
use crate::file::{
    CopyOptions, DeleteOptions, FileError, FileType, MkdirOptions, MoveOptions, WriteMode,
    WriteOptions,
};
use crate::filesystem::{
    FilesystemCapabilities, FilesystemCapability, FilesystemEntry, FilesystemEntryPage,
    FilesystemMutation, FilesystemMutationContext, FilesystemPageRequest,
};
use russh::client::Handler;
use russh::keys::{check_known_hosts_path, PrivateKeyWithHashAlg, PublicKey};
use russh_sftp::client::{error::Error as SftpError, SftpSession};
use russh_sftp::protocol::{FileType as SftpFileType, OpenFlags, StatusCode};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Explicit credentials for one native SFTP connection.
#[derive(Clone)]
pub enum SftpAuthentication {
    Password(String),
    PrivateKey(PrivateKeyWithHashAlg),
}

/// Fail-closed server host-key policy.
#[derive(Clone)]
pub enum SftpHostKeyPolicy {
    Pinned(Vec<PublicKey>),
    KnownHosts(PathBuf),
}

/// Host-owned options for opening one authenticated SFTP transport.
#[derive(Clone)]
pub struct SftpConnectOptions {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub authentication: SftpAuthentication,
    pub host_key_policy: SftpHostKeyPolicy,
    pub timeout: Duration,
}

impl SftpConnectOptions {
    fn validate(&self) -> Result<(), FileError> {
        if self.host.trim().is_empty() || self.host.contains('/') || self.host.contains('\0') {
            return Err(FileError::InvalidPath("SFTP host is invalid".into()));
        }
        if self.port == 0 || self.username.trim().is_empty() || self.timeout.is_zero() {
            return Err(FileError::InvalidPath(
                "SFTP connection options are invalid".into(),
            ));
        }
        if matches!(&self.host_key_policy, SftpHostKeyPolicy::Pinned(keys) if keys.is_empty()) {
            return Err(FileError::PermissionDenied);
        }
        Ok(())
    }
}

/// A native SFTP client backed by a dedicated protocol worker.
pub struct NativeSftpClient {
    sender: mpsc::Sender<Request>,
    closed: Arc<AtomicBool>,
    capabilities: FilesystemCapabilities,
}

enum Request {
    Stat(String, Reply<FilesystemEntry>),
    Read(String, Reply<Vec<u8>>),
    Write(String, Vec<u8>, WriteOptions, Reply<FilesystemMutation>),
    Entries(String, FilesystemPageRequest, Reply<FilesystemEntryPage>),
    Mkdir(String, MkdirOptions, Reply<FilesystemMutation>),
    Delete(String, DeleteOptions, Reply<FilesystemMutation>),
    Copy(String, String, CopyOptions, Reply<FilesystemMutation>),
    Move(String, String, MoveOptions, Reply<FilesystemMutation>),
    Close(Reply<()>),
}

type Reply<T> = mpsc::Sender<Result<T, FileError>>;

impl NativeSftpClient {
    pub fn connect(options: SftpConnectOptions) -> Result<Self, FileError> {
        options.validate()?;
        let (sender, receiver) = mpsc::channel();
        let (ready_sender, ready_receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("hara-sftp".into())
            .spawn(move || worker(options, receiver, ready_sender))
            .map_err(|error| FileError::Io(format!("could not start SFTP worker: {error}")))?;
        let capabilities = ready_receiver
            .recv()
            .map_err(|_| FileError::Io("SFTP worker stopped during connection".into()))??;
        Ok(Self {
            sender,
            closed: Arc::new(AtomicBool::new(false)),
            capabilities,
        })
    }

    fn request<T>(&self, build: impl FnOnce(Reply<T>) -> Request) -> Result<T, FileError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(FileError::Io("SFTP client is closed".into()));
        }
        let (reply_sender, reply_receiver) = mpsc::channel();
        self.sender
            .send(build(reply_sender))
            .map_err(|_| FileError::Io("SFTP worker is unavailable".into()))?;
        reply_receiver
            .recv()
            .map_err(|_| FileError::Io("SFTP worker dropped the response".into()))?
    }
}

impl RemoteFilesystemClient for NativeSftpClient {
    fn authenticated(&self) -> bool {
        true
    }

    fn host_key_verified(&self) -> bool {
        true
    }

    fn capabilities(&self) -> FilesystemCapabilities {
        self.capabilities.clone()
    }

    fn stat(&self, path: &str) -> Result<FilesystemEntry, FileError> {
        self.request(|reply| Request::Stat(path.into(), reply))
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, FileError> {
        self.request(|reply| Request::Read(path.into(), reply))
    }

    fn write(
        &self,
        path: &str,
        bytes: Vec<u8>,
        options: WriteOptions,
        _mutation: &FilesystemMutationContext,
    ) -> Result<FilesystemMutation, FileError> {
        self.request(|reply| Request::Write(path.into(), bytes, options, reply))
    }

    fn entries_page(
        &self,
        path: &str,
        request: &FilesystemPageRequest,
    ) -> Result<FilesystemEntryPage, FileError> {
        self.request(|reply| Request::Entries(path.into(), request.clone(), reply))
    }

    fn mkdir(
        &self,
        path: &str,
        options: MkdirOptions,
        _mutation: &FilesystemMutationContext,
    ) -> Result<FilesystemMutation, FileError> {
        self.request(|reply| Request::Mkdir(path.into(), options, reply))
    }

    fn delete(
        &self,
        path: &str,
        options: DeleteOptions,
        _mutation: &FilesystemMutationContext,
    ) -> Result<FilesystemMutation, FileError> {
        self.request(|reply| Request::Delete(path.into(), options, reply))
    }

    fn copy(
        &self,
        source: &str,
        target: &str,
        options: CopyOptions,
        _mutation: &FilesystemMutationContext,
    ) -> Result<FilesystemMutation, FileError> {
        self.request(|reply| Request::Copy(source.into(), target.into(), options, reply))
    }

    fn move_entry(
        &self,
        source: &str,
        target: &str,
        options: MoveOptions,
        _mutation: &FilesystemMutationContext,
    ) -> Result<FilesystemMutation, FileError> {
        self.request(|reply| Request::Move(source.into(), target.into(), options, reply))
    }

    fn close(&self) -> Result<(), FileError> {
        if self.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let (reply_sender, reply_receiver) = mpsc::channel();
        self.sender
            .send(Request::Close(reply_sender))
            .map_err(|_| FileError::Io("SFTP worker is unavailable".into()))?;
        reply_receiver
            .recv()
            .map_err(|_| FileError::Io("SFTP worker dropped the close response".into()))?
    }
}

impl Drop for NativeSftpClient {
    fn drop(&mut self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let (reply_sender, _reply_receiver) = mpsc::channel();
        let _ = self.sender.send(Request::Close(reply_sender));
    }
}

struct ClientHandler {
    host: String,
    port: u16,
    policy: SftpHostKeyPolicy,
}

impl Handler for ClientHandler {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        server_public_key: &PublicKey,
    ) -> impl std::future::Future<Output = Result<bool, Self::Error>> + Send {
        let accepted = match &self.policy {
            SftpHostKeyPolicy::Pinned(keys) => keys.iter().any(|key| key == server_public_key),
            SftpHostKeyPolicy::KnownHosts(path) => {
                check_known_hosts_path(&self.host, self.port, server_public_key, path)
                    .unwrap_or(false)
            }
        };
        async move { Ok(accepted) }
    }
}

fn worker(
    options: SftpConnectOptions,
    receiver: mpsc::Receiver<Request>,
    ready_sender: mpsc::Sender<Result<FilesystemCapabilities, FileError>>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = ready_sender.send(Err(FileError::Io(format!(
                "could not start SFTP runtime: {error}"
            ))));
            return;
        }
    };
    match runtime.block_on(connect_session(&options)) {
        Ok((mut session, sftp, capabilities)) => {
            let _ = ready_sender.send(Ok(capabilities));
            runtime.block_on(run_requests(&mut session, sftp, receiver));
        }
        Err(error) => {
            let _ = ready_sender.send(Err(error));
        }
    }
}

async fn connect_session(
    options: &SftpConnectOptions,
) -> Result<
    (
        russh::client::Handle<ClientHandler>,
        SftpSession,
        FilesystemCapabilities,
    ),
    FileError,
> {
    let handler = ClientHandler {
        host: options.host.clone(),
        port: options.port,
        policy: options.host_key_policy.clone(),
    };
    let address = format!("{}:{}", options.host, options.port);
    let mut session = tokio::time::timeout(
        options.timeout,
        russh::client::connect(Arc::new(russh::client::Config::default()), address, handler),
    )
    .await
    .map_err(|_| FileError::Io("SFTP connection timed out".into()))?
    .map_err(|_| FileError::PermissionDenied)?;
    let authentication = match &options.authentication {
        SftpAuthentication::Password(password) => {
            session
                .authenticate_password(options.username.clone(), password.clone())
                .await
        }
        SftpAuthentication::PrivateKey(key) => {
            session
                .authenticate_publickey(options.username.clone(), key.clone())
                .await
        }
    }
    .map_err(|_| FileError::PermissionDenied)?;
    if !authentication.success() {
        return Err(FileError::PermissionDenied);
    }
    let channel = tokio::time::timeout(options.timeout, session.channel_open_session())
        .await
        .map_err(|_| FileError::Io("SFTP channel opening timed out".into()))?
        .map_err(|_| FileError::Io("could not open SFTP channel".into()))?;
    tokio::time::timeout(options.timeout, channel.request_subsystem(true, "sftp"))
        .await
        .map_err(|_| FileError::Io("SFTP subsystem request timed out".into()))?
        .map_err(|_| FileError::Io("SFTP subsystem was rejected".into()))?;
    let sftp = tokio::time::timeout(options.timeout, SftpSession::new(channel.into_stream()))
        .await
        .map_err(|_| FileError::Io("SFTP initialization timed out".into()))?
        .map_err(map_sftp_error)?;
    let capabilities = FilesystemCapabilities::new([
        FilesystemCapability::Read,
        FilesystemCapability::Write,
        FilesystemCapability::Entries,
        FilesystemCapability::Mkdir,
        FilesystemCapability::Delete,
        FilesystemCapability::Copy,
        FilesystemCapability::Move,
        FilesystemCapability::Append,
    ]);
    Ok((session, sftp, capabilities))
}

async fn run_requests(
    session: &mut russh::client::Handle<ClientHandler>,
    sftp: SftpSession,
    receiver: mpsc::Receiver<Request>,
) {
    while let Ok(request) = receiver.recv() {
        match request {
            Request::Stat(path, reply) => {
                let _ = reply.send(remote_stat(&sftp, &path).await);
            }
            Request::Read(path, reply) => {
                let _ = reply.send(remote_read(&sftp, &path).await);
            }
            Request::Write(path, bytes, options, reply) => {
                let _ = reply.send(remote_write(&sftp, &path, bytes, options).await);
            }
            Request::Entries(path, request, reply) => {
                let _ = reply.send(remote_entries(&sftp, &path, &request).await);
            }
            Request::Mkdir(path, options, reply) => {
                let _ = reply.send(remote_mkdir(&sftp, &path, options).await);
            }
            Request::Delete(path, options, reply) => {
                let _ = reply.send(remote_delete(&sftp, &path, options).await);
            }
            Request::Copy(source, target, options, reply) => {
                let _ = reply.send(remote_copy(&sftp, &source, &target, options).await);
            }
            Request::Move(source, target, options, reply) => {
                let _ = reply.send(remote_move(&sftp, &source, &target, options).await);
            }
            Request::Close(reply) => {
                let result = match sftp.close().await.map_err(map_sftp_error) {
                    Ok(()) => session
                        .disconnect(russh::Disconnect::ByApplication, "closed", "")
                        .await
                        .map_err(|_| FileError::Io("could not close SFTP session".into())),
                    Err(error) => Err(error),
                };
                let _ = reply.send(result);
                break;
            }
        }
    }
}

async fn remote_stat(sftp: &SftpSession, path: &str) -> Result<FilesystemEntry, FileError> {
    let metadata = sftp.symlink_metadata(path).await.map_err(map_sftp_error)?;
    Ok(entry(path, metadata))
}

async fn remote_read(sftp: &SftpSession, path: &str) -> Result<Vec<u8>, FileError> {
    let mut file = sftp.open(path).await.map_err(map_sftp_error)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .await
        .map_err(|error| FileError::Io(format!("SFTP read failed: {error}")))?;
    file.shutdown()
        .await
        .map_err(|error| FileError::Io(format!("SFTP read close failed: {error}")))?;
    Ok(bytes)
}

async fn remote_write(
    sftp: &SftpSession,
    path: &str,
    bytes: Vec<u8>,
    options: WriteOptions,
) -> Result<FilesystemMutation, FileError> {
    let flags = match options.mode {
        WriteMode::Create => OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE,
        WriteMode::Replace => OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE,
        WriteMode::Append => OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::APPEND,
    };
    let mut file = sftp
        .open_with_flags(path, flags)
        .await
        .map_err(map_sftp_error)?;
    file.write_all(&bytes)
        .await
        .map_err(|error| FileError::Io(format!("SFTP write failed: {error}")))?;
    file.shutdown()
        .await
        .map_err(|error| FileError::Io(format!("SFTP write close failed: {error}")))?;
    Ok(FilesystemMutation::path(path))
}

async fn remote_entries(
    sftp: &SftpSession,
    path: &str,
    request: &FilesystemPageRequest,
) -> Result<FilesystemEntryPage, FileError> {
    let mut entries = Vec::new();
    let read_dir = sftp.read_dir(path).await.map_err(map_sftp_error)?;
    for item in read_dir {
        let name = item.file_name();
        if name.is_empty() || name.contains('/') || name.contains('\0') {
            return Err(FileError::InvalidPath(
                "SFTP returned an invalid entry name".into(),
            ));
        }
        let child = crate::file::logical_join(path, &name)?;
        entries.push(entry(&child, item.metadata()));
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
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
    let end = offset
        .saturating_add(request.limit.max(1))
        .min(entries.len());
    Ok(FilesystemEntryPage {
        entries: entries[offset..end].to_vec(),
        next_token: (end < entries.len()).then(|| end.to_string()),
    })
}

async fn remote_mkdir(
    sftp: &SftpSession,
    path: &str,
    _options: MkdirOptions,
) -> Result<FilesystemMutation, FileError> {
    sftp.create_dir(path).await.map_err(map_sftp_error)?;
    Ok(FilesystemMutation::path(path))
}

async fn remote_delete(
    sftp: &SftpSession,
    path: &str,
    options: DeleteOptions,
) -> Result<FilesystemMutation, FileError> {
    let metadata = match sftp.symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error)
            if options.missing_ok
                && matches!(map_sftp_error(error.clone()), FileError::NotFound) =>
        {
            return Ok(FilesystemMutation::path(path));
        }
        Err(error) => return Err(map_sftp_error(error)),
    };
    if metadata.is_dir() {
        sftp.remove_dir(path).await.map_err(map_sftp_error)?;
    } else {
        sftp.remove_file(path).await.map_err(map_sftp_error)?;
    }
    Ok(FilesystemMutation::path(path))
}

async fn remote_copy(
    sftp: &SftpSession,
    source: &str,
    target: &str,
    options: CopyOptions,
) -> Result<FilesystemMutation, FileError> {
    if options.preserve_modified {
        return Err(FileError::Unsupported);
    }
    let target_exists = match sftp.symlink_metadata(target).await {
        Ok(metadata) => {
            if metadata.is_dir() || metadata.is_symlink() {
                return Err(FileError::Unsupported);
            }
            true
        }
        Err(error) if matches!(map_sftp_error(error.clone()), FileError::NotFound) => false,
        Err(error) => return Err(map_sftp_error(error)),
    };
    if target_exists && !options.replace {
        return Err(FileError::AlreadyExists);
    }
    let bytes = remote_read(sftp, source).await?;
    remote_write(
        sftp,
        target,
        bytes,
        WriteOptions {
            mode: if target_exists {
                WriteMode::Replace
            } else {
                WriteMode::Create
            },
            parents: options.parents,
        },
    )
    .await
}

async fn remote_move(
    sftp: &SftpSession,
    source: &str,
    target: &str,
    options: MoveOptions,
) -> Result<FilesystemMutation, FileError> {
    if options.atomic || options.replace {
        return Err(FileError::Unsupported);
    }
    match sftp.symlink_metadata(target).await {
        Ok(_) => return Err(FileError::AlreadyExists),
        Err(error) if matches!(map_sftp_error(error.clone()), FileError::NotFound) => {}
        Err(error) => return Err(map_sftp_error(error)),
    }
    sftp.rename(source, target).await.map_err(map_sftp_error)?;
    Ok(FilesystemMutation::path(target))
}

fn entry(path: &str, metadata: russh_sftp::client::fs::Metadata) -> FilesystemEntry {
    let kind = match metadata.file_type() {
        SftpFileType::Dir => FileType::Directory,
        SftpFileType::File => FileType::File,
        SftpFileType::Symlink => FileType::Symlink,
        SftpFileType::Other => FileType::Other,
    };
    FilesystemEntry {
        path: path.to_owned(),
        name: crate::file::logical_name(path).unwrap_or_default(),
        kind,
        size: (kind == FileType::File).then_some(metadata.len()),
        modified_at: metadata.mtime.map(|value| value as i64 * 1000),
        id: None,
        revision: None,
        capabilities: None,
        extensions: Default::default(),
    }
}

fn map_sftp_error(error: SftpError) -> FileError {
    match error {
        SftpError::Status(status) => match status.status_code {
            StatusCode::NoSuchFile => FileError::NotFound,
            StatusCode::PermissionDenied => FileError::PermissionDenied,
            StatusCode::OpUnsupported => FileError::Unsupported,
            StatusCode::Eof => FileError::NotFound,
            StatusCode::Failure => FileError::Io("SFTP operation failed".into()),
            _ => FileError::Io("SFTP protocol operation failed".into()),
        },
        SftpError::Timeout => FileError::Io("SFTP operation timed out".into()),
        SftpError::Limited(_) => FileError::Unsupported,
        SftpError::IO(_) | SftpError::UnexpectedPacket | SftpError::UnexpectedBehavior(_) => {
            FileError::Io("SFTP transport operation failed".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::server::{Auth, Msg, Server as _, Session};
    use russh::{Channel, ChannelId};
    use russh_sftp::protocol::{Attrs, Data, File, FileAttributes, Handle, Name, Status, Version};
    use std::collections::HashMap;
    use std::net::SocketAddr;
    use tokio::net::TcpListener;

    fn options() -> SftpConnectOptions {
        SftpConnectOptions {
            host: "127.0.0.1".into(),
            port: 22,
            username: "hara".into(),
            authentication: SftpAuthentication::Password("secret".into()),
            host_key_policy: SftpHostKeyPolicy::Pinned(vec![]),
            timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn connection_options_fail_closed_before_starting_a_worker() {
        match NativeSftpClient::connect(options()) {
            Err(error) => assert_eq!(error.code(), "permission-denied"),
            Ok(_) => panic!("empty pinned host-key policy must fail closed"),
        }

        let mut invalid = options();
        invalid.host.clear();
        match NativeSftpClient::connect(invalid) {
            Err(error) => assert_eq!(error.code(), "invalid-path"),
            Ok(_) => panic!("an empty SFTP host must fail validation"),
        }
    }

    #[derive(Default)]
    struct LoopbackServer;

    struct LoopbackSshSession {
        channels: std::sync::Arc<tokio::sync::Mutex<HashMap<ChannelId, Channel<Msg>>>>,
    }

    impl Default for LoopbackSshSession {
        fn default() -> Self {
            Self {
                channels: std::sync::Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            }
        }
    }

    impl russh::server::Server for LoopbackServer {
        type Handler = LoopbackSshSession;

        fn new_client(&mut self, _peer: Option<SocketAddr>) -> Self::Handler {
            LoopbackSshSession::default()
        }
    }

    impl russh::server::Handler for LoopbackSshSession {
        type Error = russh::Error;

        async fn auth_password(
            &mut self,
            _user: &str,
            _password: &str,
        ) -> Result<Auth, Self::Error> {
            Ok(Auth::Accept)
        }

        async fn channel_open_session(
            &mut self,
            channel: Channel<Msg>,
            reply: russh::server::ChannelOpenHandle,
            _session: &mut Session,
        ) -> Result<(), Self::Error> {
            self.channels.lock().await.insert(channel.id(), channel);
            reply.accept().await;
            Ok(())
        }

        async fn subsystem_request(
            &mut self,
            channel_id: ChannelId,
            name: &str,
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            if name != "sftp" {
                session.channel_failure(channel_id)?;
                return Ok(());
            }
            let channel = self.channels.lock().await.remove(&channel_id).unwrap();
            session.channel_success(channel_id)?;
            russh_sftp::server::run(channel.into_stream(), LoopbackSftp::default()).await;
            Ok(())
        }
    }

    #[derive(Default)]
    struct LoopbackSftp {
        handles: HashMap<String, String>,
    }

    impl LoopbackSftp {
        fn attrs(path: &str) -> Result<FileAttributes, StatusCode> {
            let mut attrs = FileAttributes::default();
            match path {
                "/" => attrs.set_dir(true),
                "/probe.txt" => {
                    attrs.size = Some(11);
                    attrs.set_regular(true);
                }
                _ => return Err(StatusCode::NoSuchFile),
            }
            Ok(attrs)
        }

        fn status(id: u32) -> Status {
            Status {
                id,
                status_code: StatusCode::Ok,
                error_message: "ok".into(),
                language_tag: "en".into(),
            }
        }
    }

    impl russh_sftp::server::Handler for LoopbackSftp {
        type Error = StatusCode;

        fn unimplemented(&self) -> Self::Error {
            StatusCode::OpUnsupported
        }

        async fn init(
            &mut self,
            _version: u32,
            _extensions: HashMap<String, String>,
        ) -> Result<Version, Self::Error> {
            Ok(Version::new())
        }

        async fn open(
            &mut self,
            id: u32,
            filename: String,
            _pflags: OpenFlags,
            _attrs: FileAttributes,
        ) -> Result<Handle, Self::Error> {
            Self::attrs(&filename)?;
            self.handles.insert(filename.clone(), filename.clone());
            Ok(Handle {
                id,
                handle: filename,
            })
        }

        async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
            self.handles.remove(&handle);
            Ok(Self::status(id))
        }

        async fn read(
            &mut self,
            id: u32,
            handle: String,
            offset: u64,
            len: u32,
        ) -> Result<Data, Self::Error> {
            let path = self.handles.get(&handle).ok_or(StatusCode::Failure)?;
            let bytes = b"loopback-ok";
            if path != "/probe.txt" {
                return Err(StatusCode::NoSuchFile);
            }
            let start = usize::try_from(offset).map_err(|_| StatusCode::Failure)?;
            if start >= bytes.len() {
                return Err(StatusCode::Eof);
            }
            let end = start.saturating_add(len as usize).min(bytes.len());
            Ok(Data {
                id,
                data: bytes[start..end].to_vec(),
            })
        }

        async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
            Ok(Attrs {
                id,
                attrs: Self::attrs(&path)?,
            })
        }

        async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
            self.lstat(id, path).await
        }

        async fn realpath(&mut self, id: u32, _path: String) -> Result<Name, Self::Error> {
            Ok(Name {
                id,
                files: vec![File::dummy("/")],
            })
        }
    }

    #[test]
    fn native_transport_reads_from_a_pinned_loopback_server() {
        match std::net::TcpListener::bind(("127.0.0.1", 0)) {
            Ok(listener) => drop(listener),
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("could not probe loopback socket support: {error}"),
        }
        let (ready_sender, ready_receiver) = mpsc::channel();
        let server_thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
                let host_key = russh::keys::PrivateKey::random(
                    &mut rand::rng(),
                    russh::keys::Algorithm::Ed25519,
                )
                .unwrap();
                let host_public_key = host_key.public_key().clone();
                let config = russh::server::Config {
                    keys: vec![host_key],
                    ..Default::default()
                };
                let mut server = LoopbackServer;
                let running = server.run_on_socket(Arc::new(config), &listener);
                let handle = running.handle();
                ready_sender
                    .send((
                        listener.local_addr().unwrap().port(),
                        host_public_key,
                        handle,
                    ))
                    .unwrap();
                running.await.unwrap();
            });
        });

        let (port, host_key, server_handle) = ready_receiver.recv().unwrap();
        let client = NativeSftpClient::connect(SftpConnectOptions {
            host: "127.0.0.1".into(),
            port,
            username: "hara".into(),
            authentication: SftpAuthentication::Password("secret".into()),
            host_key_policy: SftpHostKeyPolicy::Pinned(vec![host_key]),
            timeout: Duration::from_secs(5),
        })
        .unwrap();

        let root = client.stat("/").unwrap();
        assert_eq!(root.kind, FileType::Directory);
        let file = client.stat("/probe.txt").unwrap();
        assert_eq!(file.kind, FileType::File);
        assert_eq!(file.size, Some(11));
        assert_eq!(client.read("/probe.txt").unwrap(), b"loopback-ok");

        client.close().unwrap();
        server_handle.shutdown("test complete".into());
        server_thread.join().unwrap();
    }
}
