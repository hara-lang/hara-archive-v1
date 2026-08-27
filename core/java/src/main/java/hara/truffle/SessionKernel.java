package hara.truffle;

import hara.lang.protocol.IApplicable;
import hara.lang.protocol.IComponent;
import hara.lang.protocol.IContext;
import hara.lang.protocol.IInvokeIn;
import hara.lang.protocol.IMetadata;
import java.io.IOException;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CompletionException;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ThreadFactory;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.Consumer;
import hara.truffle.InstrumentationModel.Capability;
import hara.truffle.InstrumentationModel.RuntimeBackend;
import hara.truffle.InstrumentationModel.TargetDescriptor;
import hara.truffle.InstrumentationModel.TargetHandle;
import hara.truffle.InstrumentationModel.TargetKind;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.HostAccess;
import org.graalvm.polyglot.Instrument;
import org.graalvm.polyglot.PolyglotException;
import org.graalvm.polyglot.Source;
import org.graalvm.polyglot.Value;
import org.graalvm.polyglot.io.IOAccess;
import org.graalvm.polyglot.io.ByteSequence;

/** Owns the runtime contexts shared by local and RESP clients. */
final class SessionKernel implements AutoCloseable {
  private static final ConcurrentHashMap<String, SessionKernel> EMBEDDINGS =
      new ConcurrentHashMap<>();

  static SessionKernel embedding(String token) {
    return token == null || token.isEmpty() ? null : EMBEDDINGS.get(token);
  }

  /**
   * Host authority applied when a context is created.
   *
   * <p>This policy does not include a filesystem mounted explicitly through {@link
   * SessionKernel#attachFilesystem(SessionModel.SessionId, SessionModel.SessionMountId)}. Such a
   * mount is a separately delegated, scoped resource. In-process namespace and context separation
   * remains logical isolation, not a security boundary.
   */
  static final class SessionAuthorityPolicy {
    static final SessionAuthorityPolicy ZERO =
        new SessionAuthorityPolicy(false, false, false, false, false, false);

    final boolean hostFilesystem;
    final boolean hostNetwork;
    final boolean hostProcess;
    final boolean reflection;
    final boolean packages;
    final boolean project;

    SessionAuthorityPolicy(
        boolean hostFilesystem,
        boolean hostNetwork,
        boolean hostProcess,
        boolean reflection,
        boolean packages,
        boolean project) {
      this.hostFilesystem = hostFilesystem;
      this.hostNetwork = hostNetwork;
      this.hostProcess = hostProcess;
      this.reflection = reflection;
      this.packages = packages;
      this.project = project;
    }

    static SessionAuthorityPolicy root(
        boolean allowFile, boolean allowNetwork, boolean allowProcess, HaraProject project) {
      return new SessionAuthorityPolicy(
          allowFile,
          allowNetwork,
          allowProcess,
          project != null && project.hasCapability("jvm/reflection"),
          project != null,
          project != null);
    }

    String profile() {
      return hostFilesystem || hostNetwork || hostProcess || reflection || packages || project
          ? "explicit"
          : "zero";
    }
  }

  private static final SessionModel.SessionId ROOT_ID = SessionModel.SessionId.parse("ROOT");

  private final boolean allowFile;
  private final String embeddingToken = java.util.UUID.randomUUID().toString();
  private final SessionRegistry sessionRegistry = new SessionRegistry();
  private final DevelopmentResourceCatalog developmentResources =
      new DevelopmentResourceCatalog();
  private final BundleCatalog bundleCatalog = new BundleCatalog();
  private final SandboxProviderRegistry sandboxProviderRegistry =
      new SandboxProviderRegistry();
  private final SandboxRegistry sandboxRegistry = new SandboxRegistry();
  private final ExecutorService filesystemIo;
  private final ScheduledExecutorService filesystemScheduler;
  private final FilesystemMountTable filesystemMounts;
  private final InstrumentationHub instrumentationHub = new InstrumentationHub();
  private volatile boolean instrumentationActive;

  private static final class SessionRegistry {
    final ConcurrentHashMap<String, Session> entries = new ConcurrentHashMap<>();
  }

  private static final class DevelopmentResourceCatalog {
    final ConcurrentHashMap<String, String> entries = new ConcurrentHashMap<>();
  }

  private static final class BundleCatalog {
    final ConcurrentHashMap<String, byte[]> entries = new ConcurrentHashMap<>();
  }

  private static final class SandboxProviderRegistry {
    final ConcurrentHashMap<String, SandboxProvider> entries = new ConcurrentHashMap<>();
  }

  private static final class SandboxRegistry {
    final ConcurrentHashMap<Long, Sandbox> entries = new ConcurrentHashMap<>();
    final AtomicLong nextId = new AtomicLong(1);
  }

  record FilesystemInfo(
      String kind,
      Path root,
      int attachments,
      String display,
      boolean readOnly,
      IFilesystem.Capabilities capabilities,
      String revision,
      Map<String, Object> extensions,
      boolean sourceLoadable) {
    FilesystemInfo {
      extensions = extensions == null || extensions.isEmpty() ? Map.of() : Map.copyOf(extensions);
    }
  }

  private record SandboxMount(
      HaraMountedFileSystem sourceFilesystem,
      FilesystemRuntimeBinding runtimeBinding) {}

  SessionKernel(boolean allowFile, boolean allowNetwork) {
    this(allowFile, allowNetwork, false);
  }

  SessionKernel(boolean allowFile, boolean allowNetwork, boolean allowProcess) {
    this(allowFile, allowNetwork, allowProcess, null);
  }

  SessionKernel(
      boolean allowFile, boolean allowNetwork, boolean allowProcess, HaraProject project) {
    this(
        allowFile,
        allowNetwork,
        allowProcess,
        project,
        ignored -> {
          throw new IllegalArgumentException("FILESYSTEM_CREDENTIAL_UNAVAILABLE");
        });
  }

  SessionKernel(
      boolean allowFile,
      boolean allowNetwork,
      boolean allowProcess,
      HaraProject project,
      IFilesystemFactory.CredentialResolver credentials) {
    this.allowFile = allowFile;
    this.filesystemIo =
        Executors.newCachedThreadPool(daemonThreadFactory("hara-filesystem-io-"));
    this.filesystemScheduler =
        Executors.newSingleThreadScheduledExecutor(
            daemonThreadFactory("hara-filesystem-deadline-"));
    this.filesystemMounts =
        new FilesystemMountTable(
            new IFilesystemFactory.OpenContext(
                filesystemIo,
                filesystemScheduler,
                java.util.Objects.requireNonNull(credentials, "filesystem credentials")));
    EMBEDDINGS.put(embeddingToken, this);
    registerSandboxProvider(InProcessSandboxProvider.INSTANCE);
    SessionAuthorityPolicy rootAuthority =
        SessionAuthorityPolicy.root(allowFile, allowNetwork, allowProcess, project);
    sessionRegistry.entries.put(
        ROOT_ID.value(),
        new Session(
            new SessionModel.SessionSpec(ROOT_ID, rootAuthority),
            project,
            mount -> releaseMount(ROOT_ID, mount),
            () -> instrumentationHub.cleanupSession(ROOT_ID.value()),
            false,
            embeddingToken));
  }

  private static ThreadFactory daemonThreadFactory(String prefix) {
    AtomicLong sequence = new AtomicLong(1);
    return task -> {
      Thread thread = new Thread(task, prefix + sequence.getAndIncrement());
      thread.setDaemon(true);
      return thread;
    };
  }

  Session root() {
    return require(ROOT_ID);
  }

  Session require(SessionModel.SessionId id) {
    Session session = sessionRegistry.entries.get(id.value());
    if (session == null) throw new IllegalArgumentException("NO_SESSION " + id);
    return session;
  }

  NativeInstrumentation instrumentation(SessionModel.SessionId id) {
    Session session = require(id);
    registerInstrumentationTargets(id.value());
    instrumentationActive = true;
    return new NativeInstrumentation(this, session, instrumentationHub);
  }

  boolean instrumentationActive() {
    return instrumentationActive;
  }

  TargetHandle instrumentationTarget(String sessionId, TargetKind kind) {
    return instrumentationHub.targetIfPresent(instrumentationTargetId(sessionId, kind));
  }

  void clearHbcExecution(String sessionId) {
    Session session = require(SessionModel.SessionId.parse(sessionId));
    session.clearHbcExecution();
  }

  private static String instrumentationTargetId(String sessionId, TargetKind kind) {
    return sessionId + "/" + kind;
  }

  private synchronized void registerInstrumentationTargets(String sessionId) {
    if (instrumentationHub.targetIfPresent(instrumentationTargetId(sessionId, TargetKind.INTERPRETER))
        != null) {
      return;
    }
    RuntimeBackend truffleBackend = new RuntimeBackend("java-truffle");
    registerInstrumentationTarget(
        new TargetDescriptor(
            instrumentationTargetId(sessionId, TargetKind.INTERPRETER),
            sessionId,
            TargetKind.INTERPRETER,
            truffleBackend,
            java.util.Set.of(
                Capability.EVENT_SEMANTIC_BOUNDARY,
                Capability.EVENT_EXCEPTION,
                Capability.EVENT_LIFECYCLE,
                Capability.INSPECT_SOURCE_LOCATION)));
    RuntimeBackend hbcBackend = new RuntimeBackend("java-hbc");
    registerInstrumentationTarget(
        new TargetDescriptor(
            instrumentationTargetId(sessionId, TargetKind.HBC),
            sessionId,
            TargetKind.HBC,
            hbcBackend,
            java.util.Set.of(
                Capability.EVENT_INSTRUCTION,
                Capability.EVENT_CALL,
                Capability.EVENT_EXCEPTION,
                Capability.EVENT_SUSPENSION,
                Capability.EVENT_LIFECYCLE,
                Capability.INSPECT_SOURCE_LOCATION,
                Capability.INSPECT_CURRENT_FRAME,
                Capability.INSPECT_FRAMES,
                Capability.INSPECT_LOCALS,
                Capability.INSPECT_STACK,
                Capability.INSPECT_VALUE_PREVIEW,
                Capability.INSPECT_SNAPSHOT,
                Capability.CONTROL_PAUSE,
                Capability.CONTROL_SINGLE_STEP,
                Capability.CONTROL_RESUME,
                Capability.CONTROL_SETTLE,
                Capability.CONTROL_TERMINATE)));
    RuntimeBackend wholeWasmBackend = new RuntimeBackend("java-whole-wasm");
    registerInstrumentationTarget(
        new TargetDescriptor(
            instrumentationTargetId(sessionId, TargetKind.WHOLE_WASM),
            sessionId,
            TargetKind.WHOLE_WASM,
            wholeWasmBackend,
            java.util.Set.of(
                Capability.EVENT_SEMANTIC_BOUNDARY,
                Capability.EVENT_LIFECYCLE,
                Capability.INSPECT_SOURCE_LOCATION)));
  }

  private void registerInstrumentationTarget(TargetDescriptor descriptor) {
    instrumentationHub.registerTarget(descriptor);
  }

  void refreshTruffleInstrumentation(String sessionId) {
    Session session = require(SessionModel.SessionId.parse(sessionId));
    TargetHandle target =
        instrumentationHub.targetIfPresent(
            instrumentationTargetId(sessionId, TargetKind.INTERPRETER));
    if (target != null) {
      session.setTruffleInstrumentation(instrumentationHub.hasAttachments(target));
    }
  }

  InstrumentationHub instrumentationHub() {
    return instrumentationHub;
  }

  synchronized Session create(SessionModel.SessionId id) {
    if (sessionRegistry.entries.containsKey(id.value()))
      throw new IllegalArgumentException("SESSION_EXISTS " + id);
    Session session =
        new Session(
            SessionModel.SessionSpec.zeroAuthority(id),
            null,
            mount -> releaseMount(id, mount),
            () -> instrumentationHub.cleanupSession(id.value()),
            false,
            embeddingToken);
    sessionRegistry.entries.put(id.value(), session);
    return session;
  }

  void registerFilesystemProvider(IFilesystemFactory factory) {
    filesystemMounts.register(factory);
  }

  void loadJvmProvider(JvmPackageLoader.Selection selection) {
    filesystemMounts.loadJvmProvider(selection);
  }

  CompletionStage<SessionModel.SessionMountId> createFilesystem(
      String kind, Map<String, ?> configuration) {
    if (!allowFile) {
      return java.util.concurrent.CompletableFuture.failedFuture(
          new IllegalArgumentException("FILE_ACCESS_DENIED"));
    }
    return filesystemMounts.open(kind, configuration);
  }

  SessionModel.SessionMountId createFilesystem(Path root) {
    if (!allowFile) throw new IllegalArgumentException("FILE_ACCESS_DENIED");
    return await(filesystemMounts.openNative(root));
  }

  synchronized void attachFilesystem(
      SessionModel.SessionId sessionId, SessionModel.SessionMountId mountId) {
    if (!allowFile) throw new IllegalArgumentException("FILE_ACCESS_DENIED");
    Session session = require(sessionId);
    FilesystemMountTable.AttachmentKey key =
        FilesystemMountTable.AttachmentKey.session(sessionId);
    filesystemMounts.attach(
        key,
        mountId,
        opened -> {
          FilesystemRuntimeBinding runtime =
              new FilesystemRuntimeBinding(opened.filesystem());
          try {
            session.attachFilesystem(
                new Session.AttachedFilesystem(
                    mountId, runtime, opened.graalFilesystem()));
          } catch (Throwable error) {
            runtime.close();
            throw error;
          }
        });
    refreshTruffleInstrumentation(sessionId.value());
  }

  synchronized void detachFilesystem(SessionModel.SessionId sessionId) {
    Session session = require(sessionId);
    FilesystemMountTable.AttachmentKey key =
        FilesystemMountTable.AttachmentKey.session(sessionId);
    filesystemMounts.detach(
        key,
        expected -> {
          SessionModel.SessionMountId released = session.detachFilesystem();
          if (!expected.equals(released)) {
            throw new IllegalStateException(
                "FILESYSTEM_ATTACHMENT_MISMATCH " + sessionId + " " + expected);
          }
        });
    refreshTruffleInstrumentation(sessionId.value());
  }

  SessionModel.SessionMountId filesystem(SessionModel.SessionId sessionId) {
    return require(sessionId).filesystemMount();
  }

  FilesystemRuntimeBinding filesystemRuntime(SessionModel.SessionId sessionId) {
    return require(sessionId).filesystemRuntime();
  }

  synchronized FilesystemInfo filesystemInfo(SessionModel.SessionMountId mountId) {
    FilesystemMountTable.Info info = filesystemMounts.info(mountId);
    IFilesystem.Descriptor descriptor = info.descriptor();
    HaraMountedFileSystem source = filesystemMounts.graalFilesystem(mountId);
    return new FilesystemInfo(
        descriptor.kind(),
        source == null ? null : source.root(),
        info.attachments(),
        descriptor.display(),
        descriptor.readOnly(),
        descriptor.capabilities(),
        descriptor.revision(),
        descriptor.extensions(),
        info.sourceLoadable());
  }

  synchronized void closeFilesystem(SessionModel.SessionMountId mountId) {
    await(filesystemMounts.close(mountId));
  }

  synchronized void mountFilesystem(SessionModel.SessionId sessionId, Path root) {
    SessionModel.SessionMountId previous = filesystem(sessionId);
    SessionModel.SessionMountId created = createFilesystem(root);
    try {
      attachFilesystem(sessionId, created);
    } catch (RuntimeException error) {
      closeFilesystem(created);
      throw error;
    }
    if (previous != null) closeFilesystem(previous);
  }

  private synchronized void releaseMount(
      SessionModel.SessionId sessionId, SessionModel.SessionMountId mountId) {
    SessionModel.SessionMountId released =
        filesystemMounts.releaseAttachment(
            FilesystemMountTable.AttachmentKey.session(sessionId));
    if (released != null && !released.equals(mountId)) {
      throw new IllegalStateException(
          "FILESYSTEM_ATTACHMENT_MISMATCH " + sessionId + " " + mountId);
    }
  }

  synchronized void closeSession(SessionModel.SessionId id) {
    if (ROOT_ID.equals(id)) throw new IllegalArgumentException("ROOT_CANNOT_CLOSE");
    Session removed = sessionRegistry.entries.remove(id.value());
    if (removed == null) throw new IllegalArgumentException("NO_SESSION " + id);
    removed.close();
  }

  Set<SessionModel.SessionId> sessionIds() {
    java.util.HashSet<SessionModel.SessionId> ids = new java.util.HashSet<>();
    for (Session session : sessionRegistry.entries.values()) ids.add(session.id());
    return Collections.unmodifiableSet(ids);
  }

  int size() {
    return sessionRegistry.entries.size();
  }

  void registerDevelopmentResource(String name, String source) {
    developmentResources.entries.put(name, source);
  }

  boolean removeDevelopmentResource(String name) {
    return developmentResources.entries.remove(name) != null;
  }

  Set<String> developmentResourceNames() {
    return Collections.unmodifiableSet(new java.util.TreeSet<>(developmentResources.entries.keySet()));
  }

  synchronized void registerBundle(String digest, byte[] bytes) {
    byte[] frozen = Arrays.copyOf(bytes, bytes.length);
    byte[] current = bundleCatalog.entries.get(digest);
    if (current != null && !Arrays.equals(current, frozen)) {
      throw new IllegalArgumentException("BUNDLE_DIGEST_CONFLICT " + digest);
    }
    if (current == null) bundleCatalog.entries.put(digest, frozen);
  }

  byte[] bundle(String digest) {
    byte[] bytes = bundleCatalog.entries.get(digest);
    return bytes == null ? null : Arrays.copyOf(bytes, bytes.length);
  }

  private static final class Sandbox {
    final SandboxModel.SandboxId id;
    final String provider;
    final boolean secure;
    final SessionModel.SessionMountId mount;
    final FilesystemRuntimeBinding mountRuntime;
    final SandboxProvider.SandboxInstance instance;
    private final AtomicLong nextEvaluationId = new AtomicLong(1);

    Sandbox(
        SandboxModel.SandboxId id,
        String provider,
        boolean secure,
        SessionModel.SessionMountId mount,
        FilesystemRuntimeBinding mountRuntime,
        SandboxProvider.SandboxInstance instance) {
      this.id = id;
      this.provider = provider;
      this.secure = secure;
      this.mount = mount;
      this.mountRuntime = mountRuntime;
      this.instance = instance;
    }

    SandboxModel.EvaluationId allocateEvaluation() {
      long value = nextEvaluationId.getAndIncrement();
      if (value <= 0) throw new IllegalStateException("SANDBOX_EVALUATION_IDS_EXHAUSTED");
      return new SandboxModel.EvaluationId(value);
    }
  }

  void registerSandboxProvider(SandboxProvider provider) {
    sandboxProviderRegistry.entries.put(provider.name(), provider);
  }

  synchronized SandboxModel.SandboxId openSandbox(SandboxModel.SandboxSpec spec) {
    SandboxProvider provider = sandboxProviderRegistry.entries.get(spec.provider());
    if (provider == null) {
      throw new SandboxModel.SandboxException(
          SandboxModel.ErrorCode.PROVIDER_NOT_FOUND, spec.provider());
    }
    long value = sandboxRegistry.nextId.getAndIncrement();
    if (value <= 0) throw new IllegalStateException("SANDBOX_IDS_EXHAUSTED");
    SandboxModel.SandboxId id = new SandboxModel.SandboxId(value);
    java.util.LinkedHashMap<String, byte[]> bundles = new java.util.LinkedHashMap<>();
    for (SandboxModel.BundleReference reference : spec.bundles()) {
      byte[] bytes = bundle(reference.digest());
      if (bytes == null) {
        throw new SandboxModel.SandboxException(
            SandboxModel.ErrorCode.BUNDLE_NOT_FOUND, reference.digest());
      }
      if (!reference.digest().equals(sha256Digest(bytes))) {
        throw new SandboxModel.SandboxException(
            SandboxModel.ErrorCode.BUNDLE_DIGEST_MISMATCH, reference.digest());
      }
      bundles.put(reference.digest(), bytes);
    }

    AtomicReference<SandboxMount> mounted = new AtomicReference<>();
    if (spec.mount() != null) {
      try {
        filesystemMounts.attach(
            FilesystemMountTable.AttachmentKey.sandbox(id),
            spec.mount(),
            opened -> {
              if (opened.graalFilesystem() == null) {
                throw new SandboxModel.SandboxException(
                    SandboxModel.ErrorCode.UNSUPPORTED,
                    "in-process sandbox requires a source-loadable filesystem mount");
              }
              mounted.set(
                  new SandboxMount(
                      opened.graalFilesystem(),
                      new FilesystemRuntimeBinding(opened.filesystem())));
            });
      } catch (IllegalArgumentException error) {
        if (error.getMessage() != null && error.getMessage().startsWith("NO_FILESYSTEM")) {
          throw new SandboxModel.SandboxException(
              SandboxModel.ErrorCode.MOUNT_NOT_FOUND, spec.mount().toString());
        }
        throw error;
      }
    }

    SandboxMount mount = mounted.get();
    try {
      SandboxProvider.ResolvedSpec resolved =
          new SandboxProvider.ResolvedSpec(
              spec,
              java.util.Collections.unmodifiableMap(bundles),
              mount == null ? null : mount.sourceFilesystem(),
              mount == null ? null : mount.runtimeBinding());
      SandboxProvider.SandboxInstance instance = provider.open(resolved);
      sandboxRegistry.entries.put(
          value,
          new Sandbox(
              id,
              provider.name(),
              provider.secure(),
              spec.mount(),
              mount == null ? null : mount.runtimeBinding(),
              instance));
    } catch (RuntimeException error) {
      if (mount != null) mount.runtimeBinding().close();
      releaseSandboxMount(id, spec.mount());
      throw error;
    }
    return id;
  }

  private static String sha256Digest(byte[] bytes) {
    try {
      byte[] digest = java.security.MessageDigest.getInstance("SHA-256").digest(bytes);
      return "sha256:" + java.util.HexFormat.of().formatHex(digest);
    } catch (java.security.NoSuchAlgorithmException error) {
      throw new IllegalStateException("SHA-256 is required", error);
    }
  }

  private void releaseSandboxMount(
      SandboxModel.SandboxId sandboxId, SessionModel.SessionMountId mountId) {
    if (mountId == null) return;
    SessionModel.SessionMountId released =
        filesystemMounts.releaseAttachment(
            FilesystemMountTable.AttachmentKey.sandbox(sandboxId));
    if (released != null && !released.equals(mountId)) {
      throw new IllegalStateException(
          "FILESYSTEM_ATTACHMENT_MISMATCH " + sandboxId + " " + mountId);
    }
  }

  private Sandbox requireSandbox(SandboxModel.SandboxId id) {
    Sandbox sandbox = sandboxRegistry.entries.get(id.value());
    if (sandbox == null) {
      throw new SandboxModel.SandboxException(SandboxModel.ErrorCode.NOT_FOUND, id.toString());
    }
    return sandbox;
  }

  SandboxProvider.Pending<Object> sandboxEval(SandboxModel.SandboxId id, String source) {
    Sandbox sandbox = requireSandbox(id);
    return sandbox.instance.eval(sandbox.allocateEvaluation(), source);
  }

  SandboxProvider.Pending<Object> sandboxCall(
      SandboxModel.SandboxId id, String callable, java.util.List<Object> arguments) {
    Sandbox sandbox = requireSandbox(id);
    return sandbox.instance.call(sandbox.allocateEvaluation(), callable, arguments);
  }

  boolean cancelSandbox(SandboxModel.SandboxId id) {
    SandboxProvider.SandboxInstance instance = requireSandbox(id).instance;
    SandboxModel.EvaluationId evaluation = instance.activeEvaluation();
    return evaluation != null && instance.cancel(evaluation);
  }

  SandboxModel.SandboxStatus sandboxStatus(SandboxModel.SandboxId id) {
    Sandbox sandbox = requireSandbox(id);
    return new SandboxModel.SandboxStatus(
        sandbox.id,
        sandbox.provider,
        sandbox.instance.state(),
        sandbox.secure,
        sandbox.instance.activeEvaluation() != null,
        sandbox.instance.error());
  }

  synchronized void closeSandbox(SandboxModel.SandboxId id) {
    Sandbox sandbox = sandboxRegistry.entries.remove(id.value());
    if (sandbox == null) {
      throw new SandboxModel.SandboxException(SandboxModel.ErrorCode.NOT_FOUND, id.toString());
    }
    try {
      sandbox.instance.close();
    } finally {
      if (sandbox.mountRuntime != null) sandbox.mountRuntime.close();
      releaseSandboxMount(id, sandbox.mount);
    }
  }

  @Override
  public synchronized void close() {
    EMBEDDINGS.remove(embeddingToken, this);
    RuntimeException failure = null;
    for (Sandbox sandbox : List.copyOf(sandboxRegistry.entries.values())) {
      try {
        closeSandbox(sandbox.id);
      } catch (RuntimeException error) {
        if (failure == null) failure = error;
        else failure.addSuppressed(error);
      }
    }
    for (Session session : List.copyOf(sessionRegistry.entries.values())) {
      try {
        session.close();
      } catch (RuntimeException error) {
        if (failure == null) failure = error;
        else failure.addSuppressed(error);
      }
    }
    sessionRegistry.entries.clear();
    try {
      instrumentationHub.close();
    } catch (RuntimeException error) {
      if (failure == null) failure = error;
      else failure.addSuppressed(error);
    }
    try {
      filesystemMounts.close();
    } catch (RuntimeException error) {
      if (failure == null) failure = error;
      else failure.addSuppressed(error);
    } finally {
      filesystemIo.shutdownNow();
      filesystemScheduler.shutdownNow();
    }
    if (failure != null) throw failure;
  }

  private static <T> T await(CompletionStage<T> stage) {
    try {
      return stage.toCompletableFuture().join();
    } catch (CompletionException error) {
      Throwable cause = error.getCause();
      while ((cause instanceof CompletionException
              || cause instanceof java.util.concurrent.ExecutionException)
          && cause.getCause() != null) {
        cause = cause.getCause();
      }
      if (cause instanceof RuntimeException runtime) throw runtime;
      throw error;
    }
  }

  static final class Session
      implements AutoCloseable, IContext, IComponent, IApplicable, IInvokeIn {
    private final SessionModel.SessionSpec spec;
    private final SessionAuthorityPolicy authority;
    private final HaraProject project;
    private final Consumer<SessionModel.SessionMountId> mountRelease;
    private final Runnable instrumentationCleanup;
    private final boolean sandboxRestricted;
    private final String kernelToken;
    private Context context;
    private String contextFilesystemBindingToken;
    private volatile AttachedFilesystem filesystem;
    private final AtomicInteger activeEvaluations = new AtomicInteger();
    private HaraInstrumentation.Service truffleInstrumentation;
    private final AtomicReference<SessionModel.SessionState> state =
        new AtomicReference<>(SessionModel.SessionState.NEW);

    private record AttachedFilesystem(
        SessionModel.SessionMountId id,
        FilesystemRuntimeBinding runtime,
        HaraMountedFileSystem sourceFilesystem) {}

    private record ContextLease(Context context, String filesystemBindingToken) {}

    private Session(
        SessionModel.SessionSpec spec,
        HaraProject project,
        Consumer<SessionModel.SessionMountId> mountRelease,
        Runnable instrumentationCleanup,
        boolean sandboxRestricted,
        String kernelToken) {
      this.spec = spec;
      this.authority = spec.authority();
      this.project = project;
      this.mountRelease = mountRelease;
      this.instrumentationCleanup = instrumentationCleanup;
      this.sandboxRestricted = sandboxRestricted;
      this.kernelToken = kernelToken;
      ContextLease initial = createContext(null);
      context = initial.context();
      contextFilesystemBindingToken = initial.filesystemBindingToken();
      activate();
    }

    static Session privateSandbox(String entryNamespace) {
      Session session =
          new Session(
              SessionModel.SessionSpec.zeroAuthority(SessionModel.SessionId.parse("SANDBOX")),
              null,
              ignored -> {},
              () -> {},
              true,
              null);
      if (!"user".equals(entryNamespace)) session.eval("(ns " + entryNamespace + ")");
      return session;
    }

    void attachSandboxFilesystem(
        SessionModel.SessionMountId mountId,
        HaraMountedFileSystem sourceFilesystem,
        FilesystemRuntimeBinding runtime) {
      attachFilesystem(new AttachedFilesystem(mountId, runtime, sourceFilesystem));
    }

    private ContextLease createContext(AttachedFilesystem filesystem) {
      IOAccess.Builder io = IOAccess.newBuilder().allowHostSocketAccess(authority.hostNetwork);
      if (filesystem == null) {
        io.allowHostFileAccess(authority.hostFilesystem);
      } else if (filesystem.sourceFilesystem() == null) {
        io.allowHostFileAccess(false);
      } else {
        io.allowHostFileAccess(false).fileSystem(filesystem.sourceFilesystem());
      }
      String filesystemBindingToken =
          filesystem == null ? null : FilesystemContextBindings.publish(filesystem.runtime());
      try {
        Context.Builder builder =
            Context.newBuilder(HaraLanguage.ID)
                .option("hara.SandboxRestricted", Boolean.toString(sandboxRestricted))
                .allowCreateProcess(authority.hostProcess)
                .allowIO(io.build());
        if (kernelToken != null) builder.option("hara.KernelToken", kernelToken);
        if (kernelToken != null) builder.option("hara.SessionId", spec.id().value());
        if (filesystemBindingToken != null) {
          builder.option("hara.FilesystemBindingToken", filesystemBindingToken);
        }
        if (authority.project && project != null && filesystem == null) {
          builder.currentWorkingDirectory(project.root());
        }
        if (authority.reflection && project != null) {
          builder.allowHostAccess(HostAccess.ALL).allowHostClassLookup(name -> true);
        }
        return new ContextLease(builder.build(), filesystemBindingToken);
      } catch (Throwable error) {
        FilesystemContextBindings.discard(filesystemBindingToken);
        throw error;
      }
    }

    private static void closeContext(ContextLease lease) {
      closeContext(lease.context(), lease.filesystemBindingToken());
    }

    private static void closeContext(Context context, String filesystemBindingToken) {
      try {
        if (context != null) context.close(true);
      } finally {
        FilesystemContextBindings.discard(filesystemBindingToken);
      }
    }

    private void requireActive() {
      SessionModel.SessionState current = state.get();
      if (current == SessionModel.SessionState.CLOSED)
        throw new IllegalStateException("SESSION_CLOSED " + id());
      if (current != SessionModel.SessionState.ACTIVE)
        throw new IllegalStateException("SESSION_NOT_ACTIVE " + id() + " " + current);
    }

    void attachFilesystem(AttachedFilesystem attached) {
      requireActive();
      if (activeEvaluations.get() != 0) throw new IllegalArgumentException("SESSION_BUSY " + id());
      ContextLease replacement = createContext(attached);
      AttachedFilesystem previousFilesystem;
      synchronized (this) {
        if (state.get() != SessionModel.SessionState.ACTIVE) {
          closeContext(replacement);
          requireActive();
        }
        if (activeEvaluations.get() != 0) {
          closeContext(replacement);
          throw new IllegalArgumentException("SESSION_BUSY " + id());
        }
        Context previous = context;
        String previousToken = contextFilesystemBindingToken;
        previousFilesystem = filesystem;
        truffleInstrumentation = null;
        context = replacement.context();
        contextFilesystemBindingToken = replacement.filesystemBindingToken();
        filesystem = attached;
        closeContext(previous, previousToken);
      }
      if (previousFilesystem != null) previousFilesystem.runtime().close();
    }

    SessionModel.SessionMountId detachFilesystem() {
      requireActive();
      if (activeEvaluations.get() != 0) throw new IllegalArgumentException("SESSION_BUSY " + id());
      ContextLease replacement = createContext(null);
      AttachedFilesystem released;
      synchronized (this) {
        if (state.get() != SessionModel.SessionState.ACTIVE) {
          closeContext(replacement);
          requireActive();
        }
        if (activeEvaluations.get() != 0) {
          closeContext(replacement);
          throw new IllegalArgumentException("SESSION_BUSY " + id());
        }
        Context previous = context;
        String previousToken = contextFilesystemBindingToken;
        released = filesystem;
        truffleInstrumentation = null;
        context = replacement.context();
        contextFilesystemBindingToken = replacement.filesystemBindingToken();
        filesystem = null;
        closeContext(previous, previousToken);
      }
      if (released != null) released.runtime().close();
      return released == null ? null : released.id();
    }

    SessionModel.SessionId id() {
      return spec.id();
    }

    String name() {
      return id().value();
    }

    SessionModel.SessionState state() {
      return state.get();
    }

    SessionModel.SessionMountId filesystemMount() {
      AttachedFilesystem attached = filesystem;
      return attached == null ? null : attached.id();
    }

    FilesystemRuntimeBinding filesystemRuntime() {
      AttachedFilesystem attached = filesystem;
      return attached == null ? null : attached.runtime();
    }

    SessionAuthorityPolicy authority() {
      return authority;
    }

    Value eval(String source) {
      return eval(source, null, 1, 1);
    }

    Value evalHbc(hara.truffle.bytecode.HbcProgram program) {
      activeEvaluations.incrementAndGet();
      try {
        synchronized (this) {
          requireActive();
          Source source =
              Source.newBuilder(
                      HaraLanguage.ID,
                      ByteSequence.create(hara.truffle.bytecode.HbcCodec.encode(program)),
                      "session.hbc")
                  .mimeType(HaraLanguage.BYTECODE_MIME_TYPE)
                  .build();
          return context.eval(source);
        }
      } catch (PolyglotException error) {
        throw new IllegalArgumentException(error.getMessage(), error);
      } catch (IOException error) {
        throw new IllegalArgumentException("Unable to construct Hara bytecode source", error);
      } finally {
        activeEvaluations.decrementAndGet();
      }
    }

    Object evalTransfer(String source) {
      return transferValue(eval(source));
    }

    Object callTransfer(String callable, List<Object> arguments) {
      activeEvaluations.incrementAndGet();
      try {
        synchronized (this) {
          requireActive();
          Value function = context.eval(HaraLanguage.ID, callable);
          if (!function.canExecute()) {
            throw new IllegalArgumentException("SESSION_VAR_NOT_CALLABLE " + callable);
          }
          Value result = function.execute(arguments.toArray());
          return transferValue(result);
        }
      } catch (PolyglotException error) {
        throw new IllegalArgumentException(error.getMessage(), error);
      } finally {
        activeEvaluations.decrementAndGet();
      }
    }

    void cancelEvaluation() {
      Context active = context;
      if (active != null) active.close(true);
    }

    private static Object transferValue(Value value) {
      if (value.isNull()) return null;
      if (value.isBoolean()) return value.asBoolean();
      if (value.isString()) return value.asString();
      if (value.fitsInLong()) return value.asLong();
      if (value.fitsInDouble()) return value.asDouble();
      String display = value.toString();
      if (display.contains("#'")
          || display.contains("#atom")
          || display.contains("#<")
          || display.contains("#object")
          || display.contains("#array")
          || display.contains("#bytes")
          || display.contains("@")) {
        throw new IllegalArgumentException("SESSION_TRANSFER_REJECTED " + display);
      }
      if (value.hasIterator() && display.startsWith("#{")) {
        java.util.LinkedHashSet<Object> transferred = new java.util.LinkedHashSet<>();
        Value iterator = value.getIterator();
        while (iterator.hasIteratorNextElement()) {
          transferred.add(transferValue(iterator.getIteratorNextElement()));
        }
        return HaraPersistentValues.normalize(transferred);
      }
      if (value.hasArrayElements()) {
        java.util.ArrayList<Object> transferred = new java.util.ArrayList<>();
        for (long index = 0; index < value.getArraySize(); index++) {
          transferred.add(transferValue(value.getArrayElement(index)));
        }
        return HaraPersistentValues.normalize(transferred);
      }
      if (value.hasHashEntries()) {
        java.util.LinkedHashMap<Object, Object> transferred = new java.util.LinkedHashMap<>();
        Value entries = value.getHashEntriesIterator();
        while (entries.hasIteratorNextElement()) {
          Value entry = entries.getIteratorNextElement();
          transferred.put(
              transferValue(entry.getArrayElement(0)), transferValue(entry.getArrayElement(1)));
        }
        return HaraPersistentValues.normalize(transferred);
      }
      if (value.hasIterator()) {
        throw new IllegalArgumentException("SESSION_TRANSFER_REJECTED " + display);
      }
      try {
        Object[] forms = HaraLanguage.readAll(display, "<session-transfer>");
        if (forms.length != 1) {
          throw new IllegalArgumentException("SESSION_TRANSFER_REJECTED " + display);
        }
        return forms[0];
      } catch (RuntimeException error) {
        if (error instanceof IllegalArgumentException
            && error.getMessage() != null
            && error.getMessage().startsWith("SESSION_TRANSFER_REJECTED")) {
          throw error;
        }
        throw new IllegalArgumentException(
            "SESSION_TRANSFER_REJECTED "
                + display
                + " ("
                + error.getClass().getSimpleName()
                + ": "
                + error.getMessage()
                + ")",
            error);
      }
    }

    Value eval(String source, String file, int line, int column) {
      activeEvaluations.incrementAndGet();
      try {
        synchronized (this) {
          requireActive();
          if (file == null || file.isBlank()) return context.eval(HaraLanguage.ID, source);
          int safeLine = Math.max(1, line);
          int safeColumn = Math.max(1, column);
          StringBuilder contextual = new StringBuilder(source.length() + safeLine + safeColumn);
          contextual.append("\n".repeat(safeLine - 1));
          contextual.append(" ".repeat(safeColumn - 1));
          contextual.append(source);
          Source contextualSource =
              Source.newBuilder(HaraLanguage.ID, contextual.toString(), file).build();
          return context.eval(contextualSource);
        }

      } catch (IOException error) {
        throw new IllegalArgumentException(
            "Unable to construct Hara source: " + error.getMessage(), error);
      } catch (PolyglotException error) {
        throw new IllegalArgumentException(error.getMessage(), error);
      } finally {
        activeEvaluations.decrementAndGet();
      }
    }

    Object executeHbc(hara.truffle.bytecode.HbcProgram program) {
      activeEvaluations.incrementAndGet();
      try {
        synchronized (this) {
          requireActive();
          context.initialize(HaraLanguage.ID);
          context.enter();
          try {
            return hara.truffle.bytecode.HbcBytecodeRootNode.compile(
                    HaraLanguage.currentLanguage(), program)
                .call();
          } finally {
            context.leave();
          }
        }
      } finally {
        activeEvaluations.decrementAndGet();
      }
    }

    synchronized String currentNamespace() {
      Value value = eval("(ns-current)");
      return value.isString() ? value.asString() : value.toString();
    }

    synchronized List<String> currentSymbols() {
      Value values = eval("(current-symbols)");
      List<String> result = new ArrayList<>();
      for (long index = 0; index < values.getArraySize(); index++) {
        result.add(values.getArrayElement(index).asString());
      }
      return result;
    }

    List<Object> info() {
      String filesystem =
          this.filesystem == null
              ? (authority.hostFilesystem ? "HOST" : "DENIED")
              : this.filesystem.id().toString();
      return List.of(
          "NAME", name(),
          "STATE", state().toString().toUpperCase(java.util.Locale.ROOT),
          "FILESYSTEM", filesystem,
          "AUTHORITY", authority.profile());
    }

    @Override
    public Object call(Object... args) {
      requireActive();
      if (args == null || args.length != 1 || !(args[0] instanceof String)) {
        throw new IllegalArgumentException("SESSION_CALL_EXPECTS_SOURCE " + id());
      }
      return evalTransfer((String) args[0]);
    }

    @Override
    public IMetadata getProps() {
      return metadata();
    }

    @Override
    public IMetadata getStatus() {
      return metadata();
    }

    private SessionModel.SessionStatus metadata() {
      boolean running = state.get() == SessionModel.SessionState.ACTIVE;
      return new SessionModel.SessionStatus(
          id(), running ? currentNamespace() : null, state(), filesystemMount(), authority);
    }

    @Override
    public boolean isStarted() {
      return state.get() == SessionModel.SessionState.ACTIVE;
    }

    @Override
    public boolean isStopped() {
      return state.get() == SessionModel.SessionState.CLOSED;
    }

    @Override
    public IComponent start() {
      requireActive();
      return this;
    }

    @Override
    public IComponent stop() {
      close();
      return this;
    }

    @Override
    public Object applyDefault() {
      return this;
    }

    @Override
    public Object applyIn(Object runtime, Object[] args) {
      requireActive();
      if (!(runtime instanceof IContext)) {
        throw new IllegalArgumentException("SESSION_APPLY_EXPECTS_CONTEXT " + id());
      }
      return ((IContext) runtime).call(args == null ? new Object[0] : args);
    }

    @Override
    public Object transformIn(Object runtime, Object[] args) {
      return args;
    }

    @Override
    public Object transformOut(Object runtime, Object[] args, Object value) {
      return value;
    }

    @Override
    public Object invokeIn(IContext context, Object... args) {
      return applyIn(context, args);
    }

    @Override
    public void close() {
      if (!state.compareAndSet(SessionModel.SessionState.ACTIVE, SessionModel.SessionState.CLOSED))
        return;
      Context ownedContext;
      String ownedToken;
      AttachedFilesystem ownedFilesystem;
      synchronized (this) {
        ownedContext = context;
        ownedToken = contextFilesystemBindingToken;
        ownedFilesystem = filesystem;
        context = null;
        contextFilesystemBindingToken = null;
        filesystem = null;
      }
      try {
        closeContext(ownedContext, ownedToken);
      } finally {
        try {
          if (ownedFilesystem != null) {
            ownedFilesystem.runtime().close();
            mountRelease.accept(ownedFilesystem.id());
          }
        } finally {
          instrumentationCleanup.run();
        }
      }
    }

    private void activate() {
      if (!state.compareAndSet(SessionModel.SessionState.NEW, SessionModel.SessionState.ACTIVE)) {
        throw new IllegalStateException("SESSION_ALREADY_STARTED " + id());
      }
    }

    synchronized void setTruffleInstrumentation(boolean enabled) {
        if (!enabled) {
          if (truffleInstrumentation != null) truffleInstrumentation.deactivate();
          return;
        }
        if (context == null) return;
        if (truffleInstrumentation == null) {
          Instrument instrument = context.getEngine().getInstruments().get("hara-execution");
          if (instrument == null) {
            throw new IllegalStateException("HARA_EXECUTION_INSTRUMENT_UNAVAILABLE");
          }
          truffleInstrumentation = instrument.lookup(HaraInstrumentation.Service.class);
          if (truffleInstrumentation == null) {
            throw new IllegalStateException("HARA_EXECUTION_INSTRUMENT_SERVICE_UNAVAILABLE");
          }
        }
        truffleInstrumentation.activate();
    }

    synchronized boolean truffleInstrumentationActive() {
        return truffleInstrumentation != null && truffleInstrumentation.isActive();
    }

    synchronized void clearHbcExecution() {
        if (context == null) return;
        context.enter();
        try {
          HaraLanguage.currentContext().clearHbcContinuation();
        } finally {
          context.leave();
        }
    }
  }
}
