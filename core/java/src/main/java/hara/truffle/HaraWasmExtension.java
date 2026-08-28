package hara.truffle;

import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.security.SecureRandom;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.Executors;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.TimeUnit;
import org.graalvm.nativeimage.ImageInfo;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.Source;
import org.graalvm.polyglot.Value;
import org.graalvm.polyglot.io.ByteSequence;
import org.graalvm.polyglot.proxy.ProxyExecutable;
import org.graalvm.polyglot.proxy.ProxyObject;

/** Generic Wasm extension instance with the portable Hara host imports. */
final class HaraWasmExtension implements HaraExtensionRuntime {
  static final String HTA_PROVIDER_EVENT_SCHEMA = "hara.hta.provider.event/0-alpha";
  private static final long DEFAULT_HTA_TIMEOUT_MILLIS = 120_000L;
  private static final long MAX_WASM_MEMORY_BYTES = 64L * 1024 * 1024;
  private static final long MAX_FRAME_BYTES = 64L * 1024 * 1024;
  private static final SecureRandom RANDOM = new SecureRandom();
  private static final long NANO_ORIGIN = System.nanoTime();

  private final HaraExtensionManifest manifest;
  private final Context context;
  private final Map<String, Value> exports;
  private final Value memory;
  private final Value allocator;
  private final HaraWasmMemoryExecutor memoryExecutor;
  private final boolean hta;
  private final Set<String> capabilities;
  private final Value deallocator;
  private final Value htaStart;
  private final Value htaNextEvent;
  private final Value htaDeliver;
  private final Value htaCancel;
  private final Value htaDropTask;
  private final Value htaRelease;
  private final BlockingQueue<Command> mailbox;
  private final Map<Long, TaskFuture> tasks = new LinkedHashMap<>();
  private final ScheduledExecutorService deadlines =
      Executors.newSingleThreadScheduledExecutor(
          runnable -> {
            Thread thread = new Thread(runnable, "hara-hta-deadlines");
            thread.setDaemon(true);
            return thread;
          });
  private final Set<HtaHandle> handles = new LinkedHashSet<>();
  private final Set<Long> hostCallsSeen = new LinkedHashSet<>();
  private final Map<Long, Long> hostCallTasks = new LinkedHashMap<>();
  private final Object lifecycleLock = new Object();
  private final List<HtaProviderEvent> lifecycleEvents = new ArrayList<>();
  private long lifecycleSequence;
  private boolean lifecycleShutdown;
  private final Thread owner;

  HaraWasmExtension(HaraExtensionPackage extensionPackage) {
    manifest = extensionPackage.manifest();
    if (!"wasm".equals(manifest.provider())) {
      throw new HaraException(
          "extension/provider-unsupported: "
              + manifest.provider()
              + " for "
              + manifest.namespace());
    }
    if (!"core.v1".equals(manifest.abi())
        && !"memory.v1".equals(manifest.abi())
        && !"hta.v1".equals(manifest.abi())) {
      throw new HaraException(
          "extension/abi-unsupported: " + manifest.abi() + " for " + manifest.namespace());
    }
    boolean isHta = "hta.v1".equals(manifest.abi());
    capabilities = isHta ? supportedCapabilities() : Set.of();

    Context opened = null;
    try {
      if ((!isHta
              && (!manifest.capabilities().isEmpty()
                  || !manifest.hostCalls().isEmpty()
                  || !manifest.hostCallCapabilities().isEmpty()))
          || (isHta
              && (manifest.capabilities().stream()
                      .anyMatch(capability -> !capabilities.contains(capability))
                  || manifest.hostCallCapabilities().values().stream()
                      .flatMap(List::stream)
                      .anyMatch(capability -> !capabilities.contains(capability))))) {
        throw new HaraException(
            "extension/capability-denied: "
                + manifest.capabilities()
                + " for "
                + manifest.namespace());
      }
      byte[] bytes = extensionPackage.moduleBytes();
      byte[] libraryBytes = extensionPackage.wrappedLibraryBytes();
      HaraWasmMemoryBinding memoryBinding =
          "memory.v1".equals(manifest.abi()) ? extensionPackage.memoryBinding() : null;
      Source source =
          Source.newBuilder(
                  "wasm",
                  ByteSequence.create(bytes),
                  manifest.namespace() + "/" + manifest.module())
              .build();
      opened = Context.newBuilder("wasm").allowAllAccess(false).build();
      ProxyObject libraryImports = null;
      if (isHta && libraryBytes != null) {
        Source librarySource =
            Source.newBuilder(
                    "wasm",
                    ByteSequence.create(libraryBytes),
                    "hara/library")
                .build();
        Value libraryModule = opened.eval(librarySource);
        HtaWasmImportState libraryImportState = new HtaWasmImportState();
        Value libraryInstance =
            libraryModule.canInstantiate()
                ? libraryModule.newInstance(libraryImportState.imports(null))
                : libraryModule;
        Value libraryMembers =
            libraryInstance.hasMember("exports")
                ? libraryInstance.getMember("exports")
                : libraryInstance;
        libraryImportState.bindMemory(libraryMembers);
        libraryImports = libraryImportObject(libraryMembers);
      }
      Value instance;
      Value module = opened.eval(source);
      instance =
          module.canInstantiate()
              ? isHta
                  ? module.newInstance(new HtaWasmImportState().imports(libraryImports))
                  : module.newInstance()
              : module;
      Value members = instance.hasMember("exports") ? instance.getMember("exports") : instance;
      Value memoryValue = members.hasMember("memory") ? members.getMember("memory") : null;
      Value allocatorValue = members.hasMember("alloc") ? members.getMember("alloc") : null;
      checkMemoryLimit(memoryValue, manifest.namespace());
      boolean isMemory = memoryBinding != null;
      LinkedHashMap<String, Value> declared = new LinkedHashMap<>();
      if (!isHta && !isMemory) {
        for (Map.Entry<String, HaraExtensionManifest.Export> entry :
            manifest.exports().entrySet()) {
          Value function =
              requireExport(members, entry.getValue().wasmExport(), manifest.module());
          declared.put(entry.getKey(), function);
        }
      }
      HaraWasmMemoryExecutor configuredMemoryExecutor =
          isMemory ? new HaraWasmMemoryExecutor(manifest, memoryBinding, members) : null;
      context = opened;
      exports = Map.copyOf(declared);
      memory = memoryValue;
      allocator = isHta ? requireExport(members, "hta_alloc", manifest.module()) : allocatorValue;
      memoryExecutor = configuredMemoryExecutor;
      deallocator = isHta ? requireExport(members, "hta_dealloc", manifest.module()) : null;
      htaStart = isHta ? requireExport(members, "hta_start", manifest.module()) : null;
      htaNextEvent = isHta ? requireExport(members, "hta_next_event", manifest.module()) : null;
      htaDeliver = isHta ? requireExport(members, "hta_deliver", manifest.module()) : null;
      htaCancel = isHta ? requireExport(members, "hta_cancel", manifest.module()) : null;
      htaDropTask = isHta ? requireExport(members, "hta_drop_task", manifest.module()) : null;
      htaRelease = isHta ? requireExport(members, "hta_release", manifest.module()) : null;
      if (isHta) {
        Value version = requireExport(members, "hta_abi_version", manifest.module());
        int abiVersion = version.execute().asInt();
        if (abiVersion != 1 && abiVersion != 2 && abiVersion != 3 && abiVersion != 4) {
          throw new HaraException("extension/abi-version-unsupported: " + manifest.namespace());
        }
      }
      hta = isHta;
      mailbox = isHta ? new LinkedBlockingQueue<>() : null;
      owner = isHta ? startMailboxOwner() : null;
      if (isHta) emitLifecycle("start", null, null, "ok", null);
    } catch (HaraException error) {
      deadlines.shutdownNow();
      if (opened != null) opened.close(true);
      throw error;
    } catch (Exception error) {
      deadlines.shutdownNow();
      if (opened != null) opened.close(true);
      throw new HaraException(
          "extension/module-invalid: " + manifest.namespace() + " (" + error.getMessage() + ")");
    }
  }

  private Thread startMailboxOwner() {
    String name = "hara-hta-" + manifest.namespace();
    if (!ImageInfo.inImageRuntimeCode()) {
      return Thread.ofVirtual().name(name).start(this::runMailbox);
    }
    Thread thread = new Thread(this::runMailbox, name);
    thread.start();
    return thread;
  }

  private static Value requireExport(Value members, String name, String module) {
    Value function = members.getMember(name);
    if (function == null || !function.canExecute()) {
      throw new HaraException(
          "extension/malformed: module " + module + " has no export " + name);
    }
    return function;
  }

  private static ProxyObject libraryImportObject(Value libraryMembers) {
    if (!libraryMembers.hasMembers()) {
      throw new HaraException("extension/library-malformed: exports are not a member object");
    }
    LinkedHashMap<String, Object> exports = new LinkedHashMap<>();
    for (String name : libraryMembers.getMemberKeys()) {
      Value value = libraryMembers.getMember(name);
      if (value != null) exports.put(name, value);
    }
    return ProxyObject.fromMap(exports);
  }

  private static final class HtaWasmImportState {
    private Value memory;

    private ProxyObject imports(ProxyObject libraryImports) {
      LinkedHashMap<String, Object> imports = new LinkedHashMap<>();
      imports.put(
          "env",
          ProxyObject.fromMap(
              Map.of(
                  "hara_random_fill",
                  (ProxyExecutable) this::randomFill,
                  "hara_time_ms",
                  (ProxyExecutable) arguments -> timeMillis(),
                  "hara_time_ns",
                  (ProxyExecutable) arguments -> timeNanos())));
      if (libraryImports != null) imports.put("hara/library", libraryImports);
      return ProxyObject.fromMap(imports);
    }

    private void bindMemory(Value members) {
      Value candidate = members.hasMember("memory") ? members.getMember("memory") : null;
      if (candidate != null && candidate.hasBufferElements()) memory = candidate;
    }

    private int randomFill(Value... arguments) {
      if (arguments.length != 2 || !arguments[0].isNumber() || !arguments[1].isNumber()) return 1;
      long pointer = arguments[0].asLong();
      long length = arguments[1].asLong();
      if (pointer < 0 || length < 0 || length > Integer.MAX_VALUE || memory == null) return 1;
      if (!memory.isBufferWritable()
          || pointer > memory.getBufferSize()
          || length > memory.getBufferSize() - pointer) return 1;
      byte[] bytes = new byte[(int) length];
      RANDOM.nextBytes(bytes);
      for (int i = 0; i < bytes.length; i++) memory.writeBufferByte(pointer + i, bytes[i]);
      return 0;
    }

    private long timeMillis() {
      return System.currentTimeMillis();
    }

    private long timeNanos() {
      return System.nanoTime() - NANO_ORIGIN;
    }
  }

  boolean isHta() {
    return hta;
  }

  List<HtaProviderEvent> drainLifecycleEvents() {
    synchronized (lifecycleLock) {
      List<HtaProviderEvent> result = List.copyOf(lifecycleEvents);
      lifecycleEvents.clear();
      return result;
    }
  }

  private void emitLifecycle(
      String event, Long request, String operation, String status, String code) {
    synchronized (lifecycleLock) {
      if (lifecycleShutdown && !"shutdown".equals(event)) return;
      lifecycleEvents.add(
          new HtaProviderEvent(
              HTA_PROVIDER_EVENT_SCHEMA,
              ++lifecycleSequence,
              "graalwasm",
              event,
              request,
              operation,
              status,
              code));
    }
  }

  private void emitLifecycleShutdown(String status, String code) {
    synchronized (lifecycleLock) {
      if (lifecycleShutdown) return;
      lifecycleShutdown = true;
      lifecycleEvents.add(
          new HtaProviderEvent(
              HTA_PROVIDER_EVENT_SCHEMA,
              ++lifecycleSequence,
              "graalwasm",
              "shutdown",
              null,
              null,
              status,
              code));
    }
  }

  static final class HtaProviderEvent {
    final String schema;
    final long sequence;
    final String origin;
    final String event;
    final Long request;
    final String operation;
    final String status;
    final String code;

    HtaProviderEvent(
        String schema,
        long sequence,
        String origin,
        String event,
        Long request,
        String operation,
        String status,
        String code) {
      this.schema = schema;
      this.sequence = sequence;
      this.origin = origin;
      this.event = event;
      this.request = request;
      this.operation = operation;
      this.status = status;
      this.code = code;
    }
  }

  boolean supportsDirectImport() {
    return "wasm".equals(manifest.provider()) && "core.v1".equals(manifest.abi());
  }

  public boolean asynchronous() {
    return hta;
  }

  public CompletableFuture<Object> invokeAsync(String name, Object[] values) {
    HaraExtensionManifest.Export spec = manifest.exports().get(name);
    if (spec == null) throw new HaraException("extension/export-missing: " + name);
    if (values.length != spec.arguments().size()) {
      throw new HaraException(
          manifest.namespace() + "/" + name + " expects " + spec.arguments().size() + " arguments");
    }
    validateHandles(values);
    CompletableFuture<Object> result = new CompletableFuture<>();
    TaskFuture task = new TaskFuture(result);
    mailbox.add(new Start(spec.operation(), values.clone(), task));
    result.whenComplete(
        (value, error) -> {
          if (result.isCancelled()) mailbox.add(new Cancel(task));
        });
    return result;
  }

  public synchronized Object invoke(String name, Object[] values) {
    if (hta) throw new HaraException("hta.v1 exports are asynchronous");
    if (memoryExecutor != null) return memoryExecutor.invoke(name, values);
    HaraExtensionManifest.Export spec = manifest.exports().get(name);
    if (spec == null) throw new HaraException("extension/export-missing: " + name);
    if (values.length != spec.arguments().size()) {
      throw new HaraException(
          manifest.namespace() + "/" + name + " expects " + spec.arguments().size() + " arguments");
    }
    ArrayList<Object> arguments = new ArrayList<>();
    for (int i = 0; i < values.length; i++) {
      appendArgument(arguments, spec.arguments().get(i), values[i], name);
    }
    try {
      Object result = result(spec.returns(), exports.get(name).execute(arguments.toArray()), name);
      checkMemoryLimit(memory, manifest.namespace());
      return result;
    } catch (HaraException error) {
      throw error;
    } catch (Exception error) {
      throw new HaraException(
          "extension/invoke-failed: "
              + manifest.namespace()
              + "/"
              + name
              + " ("
              + error.getMessage()
              + ")");
    }
  }

  private void appendArgument(
      ArrayList<Object> arguments, String type, Object value, String export) {
    Object input = HaraBox.unwrap(value);
    if ("utf8".equals(type)) {
      if (!(input instanceof String)) throw typeError(export, type);
      if (memory == null || allocator == null || !memory.hasBufferElements()) {
        throw new HaraException("extension/abi-memory-unavailable: " + manifest.namespace());
      }
      byte[] bytes = ((String) input).getBytes(StandardCharsets.UTF_8);
      long pointer = allocator.execute(bytes.length).asLong();
      if (pointer < 0 || pointer > Integer.MAX_VALUE) {
        throw new HaraException("extension/abi-memory-overflow: " + manifest.namespace());
      }
      try {
        if (!memory.isBufferWritable() || memory.getBufferSize() < pointer + bytes.length) {
          throw new HaraException("WASM memory is not writable or is too small");
        }
        for (int i = 0; i < bytes.length; i++) {
          memory.writeBufferByte(pointer + i, bytes[i]);
        }
      } catch (Exception error) {
        throw new HaraException(
            "extension/abi-memory-write-failed: "
                + manifest.namespace()
                + " ("
                + error.getMessage()
                + ")");
      }
      arguments.add((int) pointer);
      arguments.add(bytes.length);
      return;
    }
    if ("boolean".equals(type)) {
      if (!(input instanceof Boolean)) throw typeError(export, type);
      arguments.add((Boolean) input ? 1 : 0);
      return;
    }
    if (!HaraNumericConversions.isNumeric(input)) throw typeError(export, type);
    String operation = "extension/abi " + manifest.namespace() + "/" + export;
    if ("i32".equals(type)) {
      arguments.add(HaraNumericConversions.toInt(input, operation));
    } else if ("i64".equals(type)) {
      arguments.add(HaraNumericConversions.toLong(input, operation));
    } else if ("f32".equals(type)) {
      double converted = HaraNumericConversions.toDouble(input);
      float narrowed = (float) converted;
      if (!Float.isFinite(narrowed)) throw typeError(export, type);
      arguments.add(narrowed);
    } else if ("f64".equals(type)) {
      arguments.add(HaraNumericConversions.toDouble(input));
    } else {
      throw new HaraException("extension/abi-type-unsupported: " + type);
    }
  }

  private Object result(String type, Value value, String export) {
    if ("void".equals(type)) return HaraNull.SINGLETON;
    if ("boolean".equals(type)) return value.asInt() != 0;
    if ("i32".equals(type)) return (long) value.asInt();
    if ("i64".equals(type)) return value.asLong();
    if ("f32".equals(type)) return HaraNumericConversions.requireFinite(value.asFloat());
    if ("f64".equals(type)) return HaraNumericConversions.requireFinite(value.asDouble());
    throw new HaraException(
        "extension/abi-type-unsupported: " + manifest.namespace() + "/" + export + " -> " + type);
  }

  private HaraException typeError(String export, String expected) {
    return new HaraException(
        "extension/type-error: " + manifest.namespace() + "/" + export + " expects " + expected);
  }

  private void runMailbox() {
    try {
      boolean running = true;
      while (running) {
        Command command = mailbox.take();
        if (command instanceof Start) start((Start) command);
        else if (command instanceof Delivery) deliver((Delivery) command);
        else if (command instanceof Cancel) cancel((Cancel) command);
        else if (command instanceof Timeout) timeout((Timeout) command);
        else if (command instanceof Release) releaseNow((Release) command);
        else {
          rejectAll(new HaraException("hta/session-closed"));
          running = false;
        }
        if (running) drainEvents();
      }
    } catch (InterruptedException error) {
      Thread.currentThread().interrupt();
      rejectAll(new HaraException("hta/mailbox-interrupted"));
      emitLifecycleShutdown("error", "hta/mailbox-interrupted");
    } catch (RuntimeException error) {
      rejectAll(new HaraException("hta/mailbox-failed: " + error.getMessage()));
      emitLifecycleShutdown("error", errorCode(error));
    } finally {
      context.close(true);
    }
  }

  private void start(Start command) {
    ArrayList<Object> arguments = new ArrayList<>();
    for (Object value : command.values) {
      validateHandles(value);
      arguments.add(HaraBox.unwrap(value));
    }
    long task = executeFrame(htaStart, List.of(command.name, arguments)).asLong();
    if (task <= 0) throw new HaraException("hta/start-failed: " + manifest.namespace());
    if (tasks.containsKey(task)) {
      try {
        htaCancel.execute(task);
      } finally {
        htaDropTask.execute(task);
      }
      throw new HaraException("hta/task-duplicate: " + task);
    }
    command.result.task = task;
    command.result.operation = command.name;
    tasks.put(task, command.result);
    emitLifecycle("call-enter", task, command.name, null, null);
    long timeout = htaTimeoutMillis();
    if (timeout > 0) {
      command.result.deadline =
          deadlines.schedule(
              () -> mailbox.add(new Timeout(command.result)), timeout, TimeUnit.MILLISECONDS);
    }
    if (command.result.future.isCancelled()) {
      cancel(new Cancel(command.result));
    }
  }

  private void deliver(Delivery command) {
    Long task = hostCallTasks.get(command.call);
    if (task == null || task.longValue() != command.task || !tasks.containsKey(task)) return;
    int status =
        executeFrame(htaDeliver, List.of(command.call, command.fulfilled ? 0L : 1L, command.value))
            .asInt();
    if (status != 0) throw new HaraException("hta/deliver-failed: " + status);
    hostCallTasks.remove(command.call);
  }

  @SuppressWarnings("unchecked")
  private void drainEvents() {
    while (true) {
      long packed = htaNextEvent.execute().asLong();
      if (packed == 0) return;
      Object decoded = readFrame(packed);
      if (!(decoded instanceof List<?>)) throw new HaraException("hta/event-malformed");
      List<Object> event = (List<Object>) decoded;
      long kind = number(event, 0, "event kind");
      if (kind == 0 || kind == 1) {
        long task = number(event, 1, "task id");
        TaskFuture pending = tasks.remove(task);
        if (pending == null) continue;
        removeHostCalls(task);
        cancelDeadline(pending);
        int dropStatus = htaDropTask.execute(task).asInt();
        if (dropStatus != 0) throw new HaraException("hta/drop-task-failed: " + dropStatus);
        emitLifecycle(
            kind == 0 ? "call-return" : "call-error",
            task,
            pending.operation,
            kind == 0 ? "ok" : "error",
            null);
        if (kind == 0) pending.future.complete(event.get(2));
        else pending.future.completeExceptionally(rejection(event.get(2)));
      } else if (kind == 2) {
        hostCall(event);
      } else {
        throw new HaraException("hta/event-unknown: " + kind);
      }
    }
  }

  @SuppressWarnings("unchecked")
  private void hostCall(List<Object> event) {
    if (event.size() != 6 && event.size() != 8) {
      throw new HaraException("hta/host-call-malformed");
    }
    long call = number(event, 1, "call id");
    long task = number(event, 2, "task id");
    if (!tasks.containsKey(task)) return;
    if (!hostCallsSeen.add(call)) return;
    int serviceIndex = event.size() == 8 ? 5 : 3;
    String service = string(event.get(serviceIndex), "service");
    String method = string(event.get(serviceIndex + 1), "method");
    List<Object> arguments = (List<Object>) event.get(serviceIndex + 2);
    if (!manifest.permitsHostCall(service, method)) {
      hostCallTasks.put(call, task);
      mailbox.add(
          new Delivery(
              call, task, false, error("hta/host-call-denied", service + "/" + method)));
      return;
    }
    if (manifest.hostCallCapabilities(service, method).stream()
        .anyMatch(capability -> !capabilities.contains(capability))) {
      hostCallTasks.put(call, task);
      mailbox.add(
          new Delivery(
              call, task, false, error("hta/capability-denied", service + "/" + method)));
      return;
    }
    hostCallTasks.put(call, task);
    CompletableFuture.supplyAsync(() -> invokeHost(service, method, arguments))
        .whenComplete(
            (value, failure) ->
                mailbox.add(
                    failure == null
                        ? new Delivery(call, task, true, value)
                        : new Delivery(
                            call,
                            task,
                            false,
                            error(
                                "hta/host-call-failed",
                                failure.getCause() == null
                                    ? failure.getMessage()
                                    : failure.getCause().getMessage()))));
  }

  private Object invokeHost(String service, String method, List<Object> arguments) {
    if ("crypto.hash.sha256".equals(service) && "digest".equals(method)) {
      if (arguments.size() != 1 || !(arguments.get(0) instanceof byte[])) {
        throw new HaraException("crypto.hash.sha256/digest expects bytes");
      }
      try {
        return MessageDigest.getInstance("SHA-256").digest((byte[]) arguments.get(0));
      } catch (NoSuchAlgorithmException impossible) {
        throw new HaraException("SHA-256 is unavailable");
      }
    }
    throw new HaraException("hta/host-call-unknown: " + service + "/" + method);
  }

  private Frame writeFrame(byte[] bytes) {
    long pointer = allocator.execute(bytes.length).asLong();
    if (pointer < 0
        || pointer > Integer.MAX_VALUE
        || memory == null
        || !memory.hasBufferElements()) {
      throw new HaraException("hta/memory-unavailable: " + manifest.namespace());
    }
    checkMemoryLimit(memory, manifest.namespace());
    for (int i = 0; i < bytes.length; i++) memory.writeBufferByte(pointer + i, bytes[i]);
    return new Frame((int) pointer, bytes.length);
  }

  private Value executeFrame(Value function, Object value) {
    Frame frame = writeFrame(HtaValueCodec.encode(value));
    try {
      Value result = function.execute(frame.pointer, frame.length);
      checkMemoryLimit(memory, manifest.namespace());
      return result;
    } finally {
      deallocator.execute(frame.pointer, frame.length);
    }
  }

  private Object readFrame(long packed) {
    long pointer = packed >>> 32;
    long size = packed & 0xffff_ffffL;
    if (size > MAX_FRAME_BYTES) throw new HaraException("hta/event-size-invalid");
    if (pointer > Integer.MAX_VALUE
        || size > Integer.MAX_VALUE
        || memory.getBufferSize() < pointer + size) {
      throw new HaraException("hta/event-memory-invalid");
    }
    byte[] bytes = new byte[(int) size];
    for (int i = 0; i < bytes.length; i++) bytes[i] = memory.readBufferByte(pointer + i);
    deallocator.execute((int) pointer, bytes.length);
    return bindHandles(HtaValueCodec.decodeCanonical(bytes));
  }

  private Object bindHandles(Object value) {
    if (value instanceof HtaHandle) {
      HtaHandle handle = ((HtaHandle) value).bind(this);
      if (manifest.declaresHandles() && manifest.handleTag(handle.type()) == null) {
        throw new HaraException("hta/handle-type-denied: " + handle.type());
      }
      if (manifest.declaresHandles()
          && !manifest.namespace().equals(handle.owner())
          && (manifest.identity() == null || !manifest.identity().equals(handle.owner()))
          && !manifest.handleTag(handle.type()).equals(handle.owner())) {
        throw new HaraException("hta/handle-owner-mismatch: " + handle.owner());
      }
      String tag = manifest.handleTag(handle.type());
      if (tag != null) handle.displayAs(tag, handle.type());
      handles.add(handle);
      return handle;
    }
    if (value instanceof List<?>) ((List<?>) value).forEach(this::bindHandles);
    else if (value instanceof Set<?>) ((Set<?>) value).forEach(this::bindHandles);
    else if (value instanceof Map<?, ?>)
      ((Map<?, ?>) value)
          .forEach(
              (key, item) -> {
                bindHandles(key);
                bindHandles(item);
              });
    else if (value instanceof Iterable<?> iterable) iterable.forEach(this::bindHandles);
    else if (value instanceof Object[] array)
      for (Object item : array) bindHandles(item);
    return value;
  }

  void release(HtaHandle handle) {
    mailbox.add(new Release(handle));
  }

  private void releaseNow(Release command) {
    command.handle.requireOwner(this);
    HtaHandle wireHandle =
        new HtaHandle(command.handle.owner(), command.handle.type(), command.handle.id());
    int status;
    try {
      status = executeFrame(htaRelease, wireHandle).asInt();
    } catch (RuntimeException error) {
      emitLifecycle("release", null, null, "error", errorCode(error));
      throw error;
    }
    if (status != 0) {
      HaraException error = new HaraException("hta/handle-release-failed: " + status);
      emitLifecycle("release", null, null, "error", errorCode(error));
      throw error;
    }
    if (!handles.remove(command.handle)) {
      HaraException error = new HaraException(
          "hta/handle-stale: " + command.handle.type() + ":" + command.handle.id());
      emitLifecycle("release", null, null, "error", errorCode(error));
      throw error;
    }
    emitLifecycle("release", null, null, "ok", null);
  }

  private static long number(List<Object> values, int index, String field) {
    if (values.size() <= index || !HaraNumericConversions.isNumeric(values.get(index))) {
      throw new HaraException("hta/event-malformed: " + field);
    }
    return HaraNumericConversions.toLong(values.get(index), "hta/event " + field);
  }

  private static String string(Object value, String field) {
    if (!(value instanceof String)) throw new HaraException("hta/event-malformed: " + field);
    return (String) value;
  }

  private static Set<String> supportedCapabilities() {
    String configured =
        System.getProperty(
            "hara.hta.capabilities", System.getenv().getOrDefault("HARA_HTA_CAPABILITIES", ""));
    LinkedHashSet<String> result = new LinkedHashSet<>();
    for (String capability : configured.split("[,\\s]+")) {
      if (!capability.isBlank()) result.add(capability);
    }
    return Set.copyOf(result);
  }

  static long htaTimeoutMillis() {
    String configured = System.getProperty("hara.hta.timeout.ms");
    if (configured == null) configured = System.getenv("HARA_HTA_TIMEOUT_MS");
    if (configured == null) return DEFAULT_HTA_TIMEOUT_MILLIS;
    try {
      long timeout = Long.parseLong(configured);
      return timeout >= 0 ? timeout : DEFAULT_HTA_TIMEOUT_MILLIS;
    } catch (NumberFormatException ignored) {
      return DEFAULT_HTA_TIMEOUT_MILLIS;
    }
  }

  private static Map<Object, Object> error(String code, String message) {
    LinkedHashMap<Object, Object> error = new LinkedHashMap<>();
    error.put(hara.lang.data.Keyword.create("code"), hara.lang.data.Keyword.create(code));
    error.put(
        hara.lang.data.Keyword.create("message"), message == null ? "unknown error" : message);
    error.put(hara.lang.data.Keyword.create("origin"), hara.lang.data.Keyword.create("host"));
    error.put(hara.lang.data.Keyword.create("retryable"), false);
    return error;
  }

  private static HaraException rejection(Object value) {
    if (value instanceof Map<?, ?>) {
      Object message = ((Map<?, ?>) value).get(hara.lang.data.Keyword.create("message"));
      if (message != null) return new HaraException(String.valueOf(message));
    }
    return new HaraException("HTA task rejected: " + value);
  }

  private static String errorCode(Throwable error) {
    String message = error == null ? "provider/error" : String.valueOf(error.getMessage());
    int separator = message.indexOf(':');
    return separator > 0 ? message.substring(0, separator) : "provider/error";
  }

  private void cancel(Cancel command) {
    if (command.result.task <= 0) return;
    long task = command.result.task;
    TaskFuture pending = tasks.remove(task);
    if (pending == null) return;
    removeHostCalls(task);
    cancelDeadline(command.result);
    try {
      int status = htaCancel.execute(task).asInt();
      if (status != 0) throw new HaraException("hta/cancel-failed: " + status);
      emitLifecycle("cancel", task, pending.operation, "ok", null);
    } catch (RuntimeException error) {
      emitLifecycle("cancel", task, pending.operation, "error", errorCode(error));
      throw error;
    } finally {
      htaDropTask.execute(task);
    }
  }

  private void timeout(Timeout command) {
    if (command.result.task <= 0) return;
    long task = command.result.task;
    TaskFuture pending = tasks.remove(task);
    if (pending == null) return;
    removeHostCalls(task);
    cancelDeadline(command.result);
    try {
      htaCancel.execute(task);
    } catch (RuntimeException ignored) {
      // Match the Rust provider: cleanup failures must not change the timeout result.
    }
    try {
      htaDropTask.execute(task);
    } catch (RuntimeException ignored) {
      // The task is already removed from the host registry.
    }
    emitLifecycle("call-error", task, pending.operation, "error", "hta/timeout");
    command.result.future.completeExceptionally(new HaraException("hta/timeout"));
  }

  private static void cancelDeadline(TaskFuture task) {
    if (task.deadline != null) task.deadline.cancel(false);
  }

  private static void checkMemoryLimit(Value memory, String namespace) {
    if (memory == null || !memory.hasBufferElements()) return;
    try {
      if (memory.getBufferSize() > MAX_WASM_MEMORY_BYTES) {
        throw new HaraException("extension/resource-limit: memory exceeds the Wasm limit: " + namespace);
      }
    } catch (HaraException error) {
      throw error;
    } catch (RuntimeException error) {
      throw new HaraException(
          "extension/memory-invalid: " + namespace + " (" + error.getMessage() + ")");
    }
  }

  private void removeHostCalls(long task) {
    hostCallTasks.entrySet().removeIf(entry -> entry.getValue() == task);
  }

  private void validateHandles(Object value) {
    if (value instanceof HtaHandle handle) {
      handle.requireUsable(this);
      if (!handles.contains(handle)) {
        throw new HaraException("hta/handle-stale: " + handle.type() + ":" + handle.id());
      }
    } else if (value instanceof Map<?, ?> map) {
      map.forEach((key, item) -> {
        validateHandles(key);
        validateHandles(item);
      });
    } else if (value instanceof Iterable<?> iterable) {
      iterable.forEach(this::validateHandles);
    } else if (value instanceof Object[] array) {
      for (Object item : array) validateHandles(item);
    }
  }

  private void rejectAll(HaraException error) {
    for (long task : List.copyOf(tasks.keySet())) {
      try {
        htaCancel.execute(task);
      } catch (RuntimeException ignored) {
      }
      try {
        htaDropTask.execute(task);
      } catch (RuntimeException ignored) {
      }
    }
    tasks.values().forEach(
        task -> {
          cancelDeadline(task);
          task.future.completeExceptionally(error);
        });
    hostCallTasks.clear();
    tasks.clear();
  }

  private interface Command {}

  private static final class TaskFuture {
    private final CompletableFuture<Object> future;
    private long task;
    private String operation;
    private ScheduledFuture<?> deadline;

    private TaskFuture(CompletableFuture<Object> future) {
      this.future = future;
    }
  }

  private static final class Start implements Command {
    private final String name;
    private final Object[] values;
    private final TaskFuture result;

    private Start(String name, Object[] values, TaskFuture result) {
      this.name = name;
      this.values = values;
      this.result = result;
    }
  }

  private static final class Delivery implements Command {
    private final long call;
    private final long task;
    private final boolean fulfilled;
    private final Object value;

    private Delivery(long call, long task, boolean fulfilled, Object value) {
      this.call = call;
      this.task = task;
      this.fulfilled = fulfilled;
      this.value = value;
    }
  }

  private static final class Cancel implements Command {
    private final TaskFuture result;

    private Cancel(TaskFuture result) {
      this.result = result;
    }
  }

  private static final class Timeout implements Command {
    private final TaskFuture result;

    private Timeout(TaskFuture result) {
      this.result = result;
    }
  }

  private static final class Release implements Command {
    private final HtaHandle handle;

    private Release(HtaHandle handle) {
      this.handle = handle;
    }
  }

  private static final class Frame {
    private final int pointer;
    private final int length;

    private Frame(int pointer, int length) {
      this.pointer = pointer;
      this.length = length;
    }
  }

  private static final class Stop implements Command {}

  @Override
  public void close() {
    deadlines.shutdownNow();
    if (!hta) {
      context.close(true);
      return;
    }
    for (HtaHandle handle : List.copyOf(handles)) handle.close();
    mailbox.add(new Stop());
    try {
      owner.join();
    } catch (InterruptedException error) {
      Thread.currentThread().interrupt();
    }
    emitLifecycleShutdown("ok", null);
  }
}
