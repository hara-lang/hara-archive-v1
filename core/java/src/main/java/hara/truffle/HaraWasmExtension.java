package hara.truffle;

import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.LinkedBlockingQueue;
import org.graalvm.nativeimage.ImageInfo;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.Source;
import org.graalvm.polyglot.Value;
import org.graalvm.polyglot.io.ByteSequence;
import org.graalvm.polyglot.proxy.ProxyObject;

/** Generic, import-free Wasm extension instance. */
final class HaraWasmExtension implements HaraExtensionRuntime {
  private final HaraExtensionManifest manifest;
  private final Context context;
  private final Map<String, Value> exports;
  private final Value memory;
  private final Value allocator;
  private final HaraWasmMemoryExecutor memoryExecutor;
  private final boolean hta;
  private final Value deallocator;
  private final Value htaStart;
  private final Value htaNextEvent;
  private final Value htaDeliver;
  private final Value htaCancel;
  private final Value htaDropTask;
  private final Value htaRelease;
  private final BlockingQueue<Command> mailbox;
  private final Map<Long, CompletableFuture<Object>> tasks = new LinkedHashMap<>();
  private final Set<HtaHandle> handles = new LinkedHashSet<>();
  private final Set<Long> hostCallsSeen = new LinkedHashSet<>();
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
    if (!manifest.capabilities().isEmpty()) {
      throw new HaraException(
          "extension/capability-denied: "
              + manifest.capabilities()
              + " for "
              + manifest.namespace());
    }

    Context opened = null;
    try {
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
      ProxyObject importObject = null;
      if (libraryBytes != null) {
        Source librarySource =
            Source.newBuilder(
                    "wasm",
                    ByteSequence.create(libraryBytes),
                    "hara/library")
                .build();
        Value libraryModule = opened.eval(librarySource);
        Value libraryInstance =
            libraryModule.canInstantiate() ? libraryModule.newInstance() : libraryModule;
        Value libraryMembers =
            libraryInstance.hasMember("exports")
                ? libraryInstance.getMember("exports")
                : libraryInstance;
        importObject = ProxyObject.fromMap(Map.of("hara/library", libraryMembers));
      }
      Value instance;
      Value module = opened.eval(source);
      instance =
          module.canInstantiate()
              ? importObject == null ? module.newInstance() : module.newInstance(importObject)
              : module;
      Value members = instance.hasMember("exports") ? instance.getMember("exports") : instance;
      Value memoryValue = members.hasMember("memory") ? members.getMember("memory") : null;
      Value allocatorValue = members.hasMember("alloc") ? members.getMember("alloc") : null;
      boolean isHta = "hta.v1".equals(manifest.abi());
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
        if (abiVersion != 1 && abiVersion != 2) {
          throw new HaraException("extension/abi-version-unsupported: " + manifest.namespace());
        }
      }
      hta = isHta;
      mailbox = isHta ? new LinkedBlockingQueue<>() : null;
      owner = isHta ? startMailboxOwner() : null;
    } catch (HaraException error) {
      if (opened != null) opened.close(true);
      throw error;
    } catch (Exception error) {
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

  boolean isHta() {
    return hta;
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
      return result(spec.returns(), exports.get(name).execute(arguments.toArray()), name);
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
    if ("f32".equals(type)) return (double) value.asFloat();
    if ("f64".equals(type)) return value.asDouble();
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
    } catch (RuntimeException error) {
      rejectAll(new HaraException("hta/mailbox-failed: " + error.getMessage()));
    } finally {
      context.close(true);
    }
  }

  private void start(Start command) {
    ArrayList<Object> arguments = new ArrayList<>();
    for (Object value : command.values) arguments.add(HaraBox.unwrap(value));
    long task = executeFrame(htaStart, List.of(command.name, arguments)).asLong();
    if (task <= 0) throw new HaraException("hta/start-failed: " + manifest.namespace());
    command.result.task = task;
    tasks.put(task, command.result.future);
    if (command.result.future.isCancelled()) htaCancel.execute(task);
  }

  private void deliver(Delivery command) {
    int status =
        executeFrame(htaDeliver, List.of(command.call, command.fulfilled ? 0L : 1L, command.value))
            .asInt();
    if (status != 0) throw new HaraException("hta/deliver-failed: " + status);
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
        CompletableFuture<Object> future = tasks.remove(task);
        if (future == null) continue;
        htaDropTask.execute(task);
        if (kind == 0) future.complete(event.get(2));
        else future.completeExceptionally(rejection(event.get(2)));
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
    if (!hostCallsSeen.add(call)) return;
    int serviceIndex = event.size() == 8 ? 5 : 3;
    String service = string(event.get(serviceIndex), "service");
    String method = string(event.get(serviceIndex + 1), "method");
    List<Object> arguments = (List<Object>) event.get(serviceIndex + 2);
    if (!manifest.permitsHostCall(service, method)) {
      mailbox.add(new Delivery(call, false, error("hta/host-call-denied", service + "/" + method)));
      return;
    }
    CompletableFuture.supplyAsync(() -> invokeHost(service, method, arguments))
        .whenComplete(
            (value, failure) ->
                mailbox.add(
                    failure == null
                        ? new Delivery(call, true, value)
                        : new Delivery(
                            call,
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
    for (int i = 0; i < bytes.length; i++) memory.writeBufferByte(pointer + i, bytes[i]);
    return new Frame((int) pointer, bytes.length);
  }

  private Value executeFrame(Value function, Object value) {
    Frame frame = writeFrame(HtaValueCodec.encode(value));
    try {
      return function.execute(frame.pointer, frame.length);
    } finally {
      deallocator.execute(frame.pointer, frame.length);
    }
  }

  private Object readFrame(long packed) {
    long pointer = packed >>> 32;
    long size = packed & 0xffff_ffffL;
    if (pointer > Integer.MAX_VALUE
        || size > Integer.MAX_VALUE
        || memory.getBufferSize() < pointer + size) {
      throw new HaraException("hta/event-memory-invalid");
    }
    byte[] bytes = new byte[(int) size];
    for (int i = 0; i < bytes.length; i++) bytes[i] = memory.readBufferByte(pointer + i);
    deallocator.execute((int) pointer, bytes.length);
    return bindHandles(HtaValueCodec.decode(bytes));
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
    return value;
  }

  void release(HtaHandle handle) {
    mailbox.add(new Release(handle));
  }

  private void releaseNow(Release command) {
    command.handle.requireOwner(this);
    HtaHandle wireHandle =
        new HtaHandle(command.handle.owner(), command.handle.type(), command.handle.id());
    int status = executeFrame(htaRelease, wireHandle).asInt();
    if (status != 0) throw new HaraException("hta/handle-release-failed: " + status);
    handles.remove(command.handle);
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

  private void cancel(Cancel command) {
    if (command.result.task > 0) htaCancel.execute(command.result.task);
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
    tasks.values().forEach(future -> future.completeExceptionally(error));
    tasks.clear();
  }

  private interface Command {}

  private static final class TaskFuture {
    private final CompletableFuture<Object> future;
    private long task;

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
    private final boolean fulfilled;
    private final Object value;

    private Delivery(long call, boolean fulfilled, Object value) {
      this.call = call;
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
  }
}
