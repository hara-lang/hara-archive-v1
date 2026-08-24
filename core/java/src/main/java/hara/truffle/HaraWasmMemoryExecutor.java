package hara.truffle;

import java.nio.ByteBuffer;
import java.nio.charset.CharacterCodingException;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Set;
import java.util.TreeSet;
import org.graalvm.polyglot.Value;

/** Generic GraalVM executor for one verified {@code memory.v1} binding plan. */
final class HaraWasmMemoryExecutor {
  private static final int MAX_MEMORY_BYTES = 64 * 1024 * 1024;
  private static final int MAX_VALUE_BYTES = 16 * 1024 * 1024;
  private static final int MAX_TOTAL_INPUT_BYTES = 32 * 1024 * 1024;
  private static final int MAX_TOTAL_COPY_BYTES = MAX_TOTAL_INPUT_BYTES + MAX_VALUE_BYTES;

  private final HaraExtensionManifest manifest;
  private final HaraWasmMemoryBinding plan;
  private final Value members;
  private final Value memory;

  HaraWasmMemoryExecutor(
      HaraExtensionManifest manifest, HaraWasmMemoryBinding plan, Value members) {
    this.manifest = manifest;
    this.plan = plan;
    this.members = members;
    plan.verifyManifest(manifest);
    Value exportedMemory = members.getMember(plan.memory().export());
    if (exportedMemory == null || !exportedMemory.hasBufferElements()) {
      throw new HaraException(
          "extension/memory-missing: module does not export " + plan.memory().export());
    }
    memory = exportedMemory;
    checkMemoryLimit();
    for (HaraWasmMemoryBinding.Function function : plan.functions().values()) {
      requireFunction(function.wasmExport());
    }
    if (plan.memory().allocate() != null) requireFunction(plan.memory().allocate());
    if (plan.memory().release() != null) requireFunction(plan.memory().release());
  }

  Object invoke(String name, Object[] values) {
    HaraWasmMemoryBinding.Function function = plan.function(name);
    if (function == null) throw new HaraException("extension/export-missing: " + name);
    if (values.length != function.arguments().size()) {
      throw new HaraException(
          "extension/arity: "
              + name
              + " expects "
              + function.arguments().size()
              + " arguments, got "
              + values.length);
    }

    TreeSet<Integer> releaseAlways = new TreeSet<>();
    TreeSet<Integer> releaseOnFailure = new TreeSet<>();
    InvocationState state = new InvocationState();
    Object result = null;
    HaraException failure = null;
    try {
      result = invokeInner(function, values, releaseAlways, releaseOnFailure, state);
    } catch (HaraException error) {
      failure = error;
    } catch (RuntimeException error) {
      failure =
          new HaraException(
              "extension/invoke-failed: "
                  + manifest.namespace()
                  + "/"
                  + name
                  + " ("
                  + message(error)
                  + ")");
    }

    if (!state.callCompleted) releaseAlways.addAll(releaseOnFailure);
    HaraException cleanupFailure = releasePointers(releaseAlways);
    if (failure != null && cleanupFailure != null) {
      throw new HaraException(failure.getMessage() + "; cleanup: " + cleanupFailure.getMessage());
    }
    if (failure != null) throw failure;
    if (cleanupFailure != null) throw cleanupFailure;
    return result;
  }

  private Object invokeInner(
      HaraWasmMemoryBinding.Function function,
      Object[] values,
      Set<Integer> releaseAlways,
      Set<Integer> releaseOnFailure,
      InvocationState state) {
    ArrayList<Object> rawArguments = new ArrayList<>();
    int totalInputBytes = 0;
    int totalCopyBytes = 0;

    for (int index = 0; index < values.length; index++) {
      HaraWasmMemoryBinding.Argument argument = function.arguments().get(index);
      Object value = HaraBox.unwrap(values[index]);
      if (!argument.pointerLength()) {
        rawArguments.add(scalarArgument(function.name(), argument.type(), value));
        continue;
      }

      byte[] bytes = memoryBytes(function.name(), argument.type(), value);
      totalInputBytes = checkedAdd(totalInputBytes, bytes.length, function.name(), "input");
      if (bytes.length > MAX_VALUE_BYTES || totalInputBytes > MAX_TOTAL_INPUT_BYTES) {
        throw new HaraException(
            "extension/resource-limit: "
                + function.name()
                + " input exceeds the memory.v1 byte limit");
      }

      int pointer = 0;
      if (bytes.length != 0) {
        pointer = allocate(function.name(), bytes.length);
        if (argument.ownership() == HaraWasmMemoryBinding.Ownership.TRANSFERRED
            && pointer != 0) {
          releaseOnFailure.add(pointer);
        }
        checkMemoryLimit();
        long start = checkedRange(pointer, bytes.length, function.name());
        if (!memory.isBufferWritable()) {
          throw new HaraException(
              "extension/memory-write-failed: " + function.name() + " memory is not writable");
        }
        try {
          for (int offset = 0; offset < bytes.length; offset++) {
            memory.writeBufferByte(start + offset, bytes[offset]);
          }
        } catch (RuntimeException error) {
          throw new HaraException(
              "extension/memory-write-failed: "
                  + function.name()
                  + " ("
                  + message(error)
                  + ")");
        }
      }
      totalCopyBytes = checkedAdd(totalCopyBytes, bytes.length, function.name(), "copy");
      if (totalCopyBytes > MAX_TOTAL_COPY_BYTES) {
        throw new HaraException(
            "extension/resource-limit: "
                + function.name()
                + " exceeds the memory.v1 aggregate copy limit");
      }
      rawArguments.add(pointer);
      rawArguments.add(bytes.length);
    }

    Value rawResult;
    try {
      rawResult = requireFunction(function.wasmExport()).execute(rawArguments.toArray());
    } catch (RuntimeException error) {
      throw new HaraException(
          "extension/invoke-failed: " + function.name() + " (" + message(error) + ")");
    }
    checkMemoryLimit();
    state.callCompleted = true;
    return liftResult(function, rawResult, releaseAlways, totalCopyBytes);
  }

  private Object scalarArgument(
      String export, HaraWasmMemoryBinding.Type type, Object input) {
    String operation = "extension/abi " + manifest.namespace() + "/" + export;
    return switch (type) {
      case BOOLEAN -> {
        if (!(input instanceof Boolean)) throw typeError(export, type);
        yield (Boolean) input ? 1 : 0;
      }
      case I32 -> HaraNumericConversions.toInt(input, operation);
      case I64 -> HaraNumericConversions.toLong(input, operation);
      case F32 -> {
        double converted = HaraNumericConversions.toDouble(input);
        float narrowed = (float) converted;
        if (!Float.isFinite(narrowed)) throw typeError(export, type);
        yield narrowed;
      }
      case F64 -> HaraNumericConversions.toDouble(input);
      default -> throw typeError(export, type);
    };
  }

  private byte[] memoryBytes(
      String export, HaraWasmMemoryBinding.Type type, Object input) {
    if (type == HaraWasmMemoryBinding.Type.BYTES && input instanceof byte[]) {
      return (byte[]) input;
    }
    if (type == HaraWasmMemoryBinding.Type.STRING && input instanceof String) {
      return ((String) input).getBytes(StandardCharsets.UTF_8);
    }
    throw typeError(export, type);
  }

  private int allocate(String export, int length) {
    String allocatorName = plan.memory().allocate();
    if (allocatorName == null) {
      throw new HaraException("extension/allocator-missing: " + export);
    }
    try {
      long pointer = requireFunction(allocatorName).execute(length).asLong();
      if (pointer < 0 || pointer > Integer.MAX_VALUE) {
        throw new HaraException(
            "extension/allocator-invalid: "
                + allocatorName
                + " returned an out-of-range pointer");
      }
      return (int) pointer;
    } catch (HaraException error) {
      throw error;
    } catch (RuntimeException error) {
      throw new HaraException(
          "extension/allocator-failed: " + export + " (" + message(error) + ")");
    }
  }

  private Object liftResult(
      HaraWasmMemoryBinding.Function function,
      Value raw,
      Set<Integer> releaseAlways,
      int totalCopyBytes) {
    HaraWasmMemoryBinding.Result result = function.result();
    if (!result.packedI64()) return scalarResult(function.name(), result.type(), raw);

    long packed;
    try {
      packed = raw.asLong();
    } catch (RuntimeException error) {
      throw new HaraException(
          "extension/abi-type-unsupported: "
              + function.name()
              + " expected a packed i64 result");
    }
    long pointerValue = packed & 0xffff_ffffL;
    long lengthValue = (packed >>> 32) & 0xffff_ffffL;
    if (pointerValue > Integer.MAX_VALUE) {
      throw new HaraException(
          "extension/memory-range: " + function.name() + " pointer is out of range");
    }
    if (lengthValue > Integer.MAX_VALUE || lengthValue > MAX_VALUE_BYTES) {
      throw new HaraException(
          "extension/resource-limit: "
              + function.name()
              + " result exceeds the memory.v1 byte limit");
    }
    int pointer = (int) pointerValue;
    int length = (int) lengthValue;
    int aggregate = checkedAdd(totalCopyBytes, length, function.name(), "copy");
    if (aggregate > MAX_TOTAL_COPY_BYTES) {
      throw new HaraException(
          "extension/resource-limit: "
              + function.name()
              + " exceeds the memory.v1 aggregate copy limit");
    }
    if (result.ownership() == HaraWasmMemoryBinding.Ownership.CALLER && pointer != 0) {
      releaseAlways.add(pointer);
    }

    long start = checkedRange(pointer, length, function.name());
    byte[] bytes = new byte[length];
    try {
      for (int offset = 0; offset < length; offset++) {
        bytes[offset] = memory.readBufferByte(start + offset);
      }
    } catch (RuntimeException error) {
      throw new HaraException(
          "extension/memory-read-failed: "
              + function.name()
              + " ("
              + message(error)
              + ")");
    }
    if (result.type() == HaraWasmMemoryBinding.Type.BYTES) return bytes;
    if (result.type() == HaraWasmMemoryBinding.Type.STRING) {
      try {
        return StandardCharsets.UTF_8
            .newDecoder()
            .onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT)
            .decode(ByteBuffer.wrap(bytes))
            .toString();
      } catch (CharacterCodingException error) {
        throw new HaraException(
            "extension/utf8-invalid: " + function.name() + " (" + error.getMessage() + ")");
      }
    }
    throw new HaraException(
        "extension/abi-type-unsupported: "
            + function.name()
            + " cannot lift :"
            + result.type().keyword());
  }

  private Object scalarResult(
      String export, HaraWasmMemoryBinding.Type type, Value value) {
    try {
      return switch (type) {
        case VOID -> HaraNull.SINGLETON;
        case BOOLEAN -> value.asInt() != 0;
        case I32 -> (long) value.asInt();
        case I64 -> value.asLong();
        case F32 -> HaraNumericConversions.requireFinite(value.asFloat());
        case F64 -> HaraNumericConversions.requireFinite(value.asDouble());
        default ->
            throw new HaraException(
                "extension/abi-type-unsupported: "
                    + manifest.namespace()
                    + "/"
                    + export
                    + " -> :"
                    + type.keyword());
      };
    } catch (HaraException error) {
      throw error;
    } catch (RuntimeException error) {
      throw new HaraException(
          "extension/result-type-invalid: " + export + " (" + message(error) + ")");
    }
  }

  private HaraException releasePointers(Set<Integer> pointers) {
    if (pointers.isEmpty()) return null;
    String releaseName = plan.memory().release();
    if (releaseName == null) {
      return new HaraException("extension/release-missing: cleanup requires a release export");
    }
    Value release;
    try {
      release = requireFunction(releaseName);
    } catch (HaraException error) {
      return error;
    }

    ArrayList<String> failures = new ArrayList<>();
    for (int pointer : pointers) {
      try {
        release.execute(pointer);
      } catch (RuntimeException error) {
        failures.add(pointer + ": " + message(error));
      }
    }
    if (failures.isEmpty()) return null;
    return new HaraException("extension/release-failed: " + String.join("; ", failures));
  }

  private long checkedRange(int pointer, int length, String export) {
    if (pointer < 0) {
      throw new HaraException("extension/memory-range: " + export + " pointer is negative");
    }
    if (length < 0) {
      throw new HaraException("extension/memory-range: " + export + " length is negative");
    }
    long start = pointer;
    long end = start + (long) length;
    if (end < start || end > memory.getBufferSize()) {
      throw new HaraException(
          "extension/memory-range: "
              + export
              + " range "
              + start
              + ".."
              + end
              + " exceeds linear memory");
    }
    return start;
  }

  private int checkedAdd(int left, int right, String export, String subject) {
    long value = (long) left + right;
    if (value > Integer.MAX_VALUE) {
      throw new HaraException(
          "extension/resource-limit: " + export + " " + subject + " byte count overflow");
    }
    return (int) value;
  }

  private void checkMemoryLimit() {
    long size;
    try {
      size = memory.getBufferSize();
    } catch (RuntimeException error) {
      throw new HaraException(
          "extension/memory-invalid: " + plan.memory().export() + " (" + message(error) + ")");
    }
    if (size > MAX_MEMORY_BYTES) {
      throw new HaraException("extension/resource-limit: memory exceeds the memory.v1 limit");
    }
  }

  private Value requireFunction(String name) {
    Value function = members.getMember(name);
    if (function == null || !function.canExecute()) {
      throw new HaraException(
          "extension/export-missing: module " + manifest.module() + " has no export " + name);
    }
    return function;
  }

  private HaraException typeError(String export, HaraWasmMemoryBinding.Type type) {
    return new HaraException(
        "extension/type-error: "
            + manifest.namespace()
            + "/"
            + export
            + " expects :"
            + type.keyword());
  }

  private static String message(RuntimeException error) {
    return error.getMessage() == null ? error.getClass().getSimpleName() : error.getMessage();
  }

  private static final class InvocationState {
    private boolean callCompleted;
  }
}
