package hara.truffle;

import hara.lang.data.Keyword;
import java.io.BufferedInputStream;
import java.io.BufferedOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.EOFException;
import java.io.IOException;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicLong;

/** Lazy managed-process transport for an HTA v1 Node target. */
final class HaraProcessExtension implements HaraExtensionRuntime {
  private static final int MAX_FRAME_BYTES = 64 * 1024 * 1024;
  private final HaraExtensionPackage extensionPackage;
  private final HaraExtensionManifest manifest;
  private final AtomicLong requestIds = new AtomicLong();
  private final ConcurrentHashMap<Long, CompletableFuture<Object>> pending =
      new ConcurrentHashMap<>();
  private final Object writeLock = new Object();
  private final AtomicBoolean closed = new AtomicBoolean();
  private volatile Process process;
  private volatile DataInputStream input;
  private volatile DataOutputStream output;
  private volatile Thread reader;

  HaraProcessExtension(HaraExtensionPackage extensionPackage, boolean allowProcess) {
    this.extensionPackage = extensionPackage;
    this.manifest = extensionPackage.manifest();
    if (!"hta".equals(manifest.provider()) || !"hta.v1".equals(manifest.abi())) {
      throw new HaraException(
          "extension/provider-unsupported: "
              + manifest.provider()
              + "/"
              + manifest.abi()
              + " for "
              + manifest.namespace());
    }
    HaraExtensionManifest.Target node = manifest.target("node");
    if (node == null || !"process".equals(node.runtime())) {
      throw new HaraException("extension/target-unsupported: node for " + manifest.namespace());
    }
    if (hasUnsupportedCapabilities()) {
      throw new HaraException(
          "extension/capability-invalid: unsupported managed Node capability");
    }
    if (!allowProcess) {
      throw new HaraException(
          "extension/capability-denied: [:process] for " + manifest.namespace());
    }
  }

  private boolean hasUnsupportedCapabilities() {
    return manifest.capabilities().stream().anyMatch(value -> !"process".equals(value));
  }

  void check() {
    ensureStarted();
  }

  @Override
  public boolean asynchronous() {
    return true;
  }

  @Override
  public Object invoke(String name, Object[] values) {
    throw new HaraException("hta.v1 exports are asynchronous");
  }

  @Override
  public CompletableFuture<Object> invokeAsync(String name, Object[] values) {
    HaraExtensionManifest.Export spec = manifest.exports().get(name);
    if (spec == null) throw new HaraException("extension/export-missing: " + name);
    if (values.length != spec.arguments().size()) {
      throw new HaraException(
          manifest.namespace() + "/" + name + " expects " + spec.arguments().size() + " arguments");
    }
    ensureStarted();
    long requestId = requestIds.incrementAndGet();
    CompletableFuture<Object> result = new CompletableFuture<>();
    pending.put(requestId, result);
    try {
      ArrayList<Object> arguments = new ArrayList<>(values.length);
      Arrays.stream(values).map(HaraBox::unwrap).forEach(arguments::add);
      write(List.of("call", requestId, name, arguments));
    } catch (RuntimeException error) {
      pending.remove(requestId);
      result.completeExceptionally(error);
    }
    result.whenComplete(
        (value, failure) -> {
          if (result.isCancelled()) {
            pending.remove(requestId);
            try {
              write(List.of("cancel", requestId));
            } catch (RuntimeException ignored) {
              // The reader will reject all work if the worker has already failed.
            }
          }
        });
    return result;
  }

  private synchronized void ensureStarted() {
    if (closed.get()) throw new HaraException("hta/process-closed: " + manifest.namespace());
    if (process != null && process.isAlive()) return;
    HaraExtensionManifest.Target target = manifest.target("node");
    Path module = extensionPackage.file(target.provider());
    String node = System.getProperty("hara.node.command", "");
    if (node.isBlank()) node = System.getenv().getOrDefault("HARA_NODE", "node");
    try {
      Process started =
          new ProcessBuilder(node, module.toString())
              .directory(module.getParent().toFile())
              .redirectError(ProcessBuilder.Redirect.INHERIT)
              .start();
      DataInputStream nextInput =
          new DataInputStream(new BufferedInputStream(started.getInputStream()));
      DataOutputStream nextOutput =
          new DataOutputStream(new BufferedOutputStream(started.getOutputStream()));
      process = started;
      input = nextInput;
      output = nextOutput;
      write(List.of("handshake", 1L, manifest.namespace(), new ArrayList<>(manifest.exports().keySet())));
      Object response = read(nextInput);
      if (!(response instanceof List<?> values)
          || values.size() < 2
          || !"ready".equals(values.get(0))
          || !(values.get(1) instanceof Number)
          || ((Number) values.get(1)).longValue() != 1L) {
        throw new HaraException("hta/handshake-invalid: " + response);
      }
      reader = startReader();
    } catch (IOException | RuntimeException error) {
      stopProcess();
      if (error instanceof HaraException) throw (HaraException) error;
      throw new HaraException(
          "hta/process-start-failed: " + manifest.namespace() + " (" + error.getMessage() + ")");
    }
  }

  private Thread startReader() {
    String name = "hara-hta-process-" + manifest.namespace();
    Thread thread = new Thread(this::readResponses, name);
    thread.setDaemon(true);
    thread.start();
    return thread;
  }

  private void readResponses() {
    try {
      while (!closed.get()) dispatch(read(input));
    } catch (EOFException error) {
      failWorker(new HaraException("hta/process-closed: " + manifest.namespace()));
    } catch (IOException | RuntimeException error) {
      if (!closed.get()) {
        failWorker(
            error instanceof HaraException
                ? (HaraException) error
                : new HaraException("hta/process-failed: " + error.getMessage()));
      }
    }
  }

  private void dispatch(Object frame) {
    if (!(frame instanceof List<?> values) || values.size() < 3 || !(values.get(1) instanceof Number)) {
      throw new HaraException("hta/process-frame-malformed");
    }
    String kind = String.valueOf(values.get(0));
    long requestId = ((Number) values.get(1)).longValue();
    CompletableFuture<Object> result = pending.remove(requestId);
    if (result == null) return;
    if ("result".equals(kind)) result.complete(toHaraValue(values.get(2)));
    else if ("error".equals(kind)) result.completeExceptionally(rejection(values.get(2)));
    else result.completeExceptionally(new HaraException("hta/process-event-unknown: " + kind));
  }

  private static Object toHaraValue(Object value) {
    if (value instanceof Map<?, ?> map) {
      ArrayList<Object> entries = new ArrayList<>(map.size() * 2);
      map.forEach((key, item) -> {
        entries.add(toHaraValue(key));
        entries.add(toHaraValue(item));
      });
      return hara.lang.data.Map.Standard.from(null, entries.toArray());
    }
    if (value instanceof java.util.Set<?> set) {
      return hara.lang.data.Set.Standard.from(
          null, set.stream().map(HaraProcessExtension::toHaraValue).toArray());
    }
    if (value instanceof List<?> list) {
      return hara.lang.data.Vector.Standard.from(
          null, list.stream().map(HaraProcessExtension::toHaraValue).toArray());
    }
    return value;
  }

  private static HaraException rejection(Object value) {
    if (value instanceof Map<?, ?> error) {
      Object code = error.get(Keyword.create("code"));
      Object message = error.get(Keyword.create("message"));
      return new HaraException(
          errorText(code, "hta/remote-error") + ": " + errorText(message, value));
    }
    return new HaraException("hta/remote-error: " + value);
  }

  private static String errorText(Object value, Object fallback) {
    Object selected = value == null ? fallback : value;
    if (selected instanceof Keyword keyword) {
      return keyword.getNamespace() == null
          ? keyword.getName()
          : keyword.getNamespace() + "/" + keyword.getName();
    }
    return String.valueOf(selected);
  }

  private void write(Object value) {
    byte[] bytes = HtaValueCodec.encode(value);
    if (bytes.length > MAX_FRAME_BYTES) throw new HaraException("hta/frame-too-large");
    DataOutputStream target = output;
    if (target == null) throw new HaraException("hta/process-unavailable: " + manifest.namespace());
    try {
      synchronized (writeLock) {
        target.writeInt(bytes.length);
        target.write(bytes);
        target.flush();
      }
    } catch (IOException error) {
      throw new HaraException("hta/process-write-failed: " + error.getMessage());
    }
  }

  private static Object read(DataInputStream source) throws IOException {
    int length = source.readInt();
    if (length <= 0 || length > MAX_FRAME_BYTES) {
      throw new HaraException("hta/process-frame-size: " + length);
    }
    return HtaValueCodec.decodeCanonical(source.readNBytes(length));
  }

  private synchronized void failWorker(HaraException error) {
    stopProcess();
    pending.values().forEach(future -> future.completeExceptionally(error));
    pending.clear();
  }

  private synchronized void stopProcess() {
    Process current = process;
    process = null;
    input = null;
    output = null;
    reader = null;
    if (current != null) current.destroy();
  }

  @Override
  public void close() {
    if (!closed.compareAndSet(false, true)) return;
    try {
      if (process != null && process.isAlive()) write(List.of("shutdown"));
    } catch (RuntimeException ignored) {
      // Forced destruction below is the shutdown fallback.
    }
    HaraException failure = new HaraException("hta/process-closed: " + manifest.namespace());
    pending.values().forEach(future -> future.completeExceptionally(failure));
    pending.clear();
    stopProcess();
  }
}
