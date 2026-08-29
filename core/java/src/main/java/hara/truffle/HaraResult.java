package hara.truffle;

import hara.lang.base.Eq;
import hara.lang.base.Ex;
import hara.lang.base.G;
import hara.lang.data.Keyword;
import hara.lang.data.Tuple;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.Constant;
import hara.lang.protocol.IDeref;
import hara.lang.protocol.IDerefTimeout;
import hara.lang.protocol.IDisplay;
import hara.lang.protocol.IEquality;
import hara.lang.protocol.IExInfo;
import hara.lang.protocol.IHash;
import hara.lang.protocol.ILookup;
import hara.lang.protocol.IPromise;
import java.util.Iterator;
import java.util.Map.Entry;
import java.util.Objects;

/** A completed native Hara outcome. Context is diagnostic and is not part of identity. */
public final class HaraResult implements IDeref<Object>, IDisplay, IEquality, IHash, ILookup<Object, Object> {
  public enum Status {
    SUCCESS,
    ERROR
  }

  private static final IMapType<Object, Object> EMPTY_CONTEXT =
      hara.lang.data.Map.Standard.EMPTY;
  private static final Object MISSING = new Object();
  private static final Object TIMEOUT = new Object();
  private static final Keyword TIMEOUT_KEY = Keyword.create("timeout");
  private static final Keyword CONTEXT_KEY = Keyword.create("context");
  private static final Keyword DISPLAY_KEY = Keyword.create("display");
  private static final Keyword ERROR_CODE_KEY = Keyword.create("code");
  private static final Keyword TIMEOUT_ERROR_CODE = Keyword.create("result", "timeout");

  private final Status status;
  private final Object data;
  private final Ex.Info error;
  private final IMapType<Object, Object> context;

  private HaraResult(
      Status status, Object data, Ex.Info error, IMapType<Object, Object> context) {
    this.status = status;
    this.data = data;
    this.error = error;
    this.context = context;
  }

  public static HaraResult success(Object data) {
    return success(data, EMPTY_CONTEXT);
  }

  public static HaraResult success(Object data, Object context) {
    return new HaraResult(Status.SUCCESS, HaraBox.unwrap(data), null, contextMap(context));
  }

  public static HaraResult error(Object error) {
    return error(error, EMPTY_CONTEXT);
  }

  public static HaraResult error(Object error, Object context) {
    return new HaraResult(Status.ERROR, null, normalizeError(error), contextMap(context));
  }

  public static HaraResult synchronize(Object value) {
    return synchronize(value, null);
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  public static HaraResult synchronize(Object value, Object options) {
    Object raw = HaraBox.unwrap(value);
    SynchronizeOptions parsed = synchronizeOptions(options);

    if (raw instanceof HaraResult result) {
      return parsed.context().count() == 0 ? result : result.withContext(parsed.context());
    }

    if (parsed.timeout() != null) {
      if (raw instanceof IDerefTimeout<?> timed) {
        try {
          Object resolved = ((IDerefTimeout) timed).derefTimeout(parsed.timeout(), TIMEOUT);
          if (resolved == TIMEOUT) {
            return timeoutResult(raw, parsed.timeout(), parsed.context());
          }
          return success(resolved, parsed.context());
        } catch (Throwable error) {
          return error(error, parsed.context());
        }
      }
      if (raw instanceof IDeref<?>) {
        return timeoutUnsupportedResult(parsed.timeout(), parsed.context());
      }
      return success(raw, parsed.context());
    }

    if (raw instanceof IDeref<?> dereferenceable) {
      try {
        return success(dereferenceable.deref(), parsed.context());
      } catch (Throwable error) {
        return error(error, parsed.context());
      }
    }
    return success(raw, parsed.context());
  }

  private static SynchronizeOptions synchronizeOptions(Object options) {
    Object raw = HaraBox.unwrap(options);
    if (raw == null) return new SynchronizeOptions(null, EMPTY_CONTEXT);
    if (!(raw instanceof IMapType<?, ?>)) {
      throw new HaraException("std.native.Result/synchronize expects an options map");
    }
    @SuppressWarnings("unchecked")
    IMapType<Object, Object> map = (IMapType<Object, Object>) raw;

    Object timeoutValue = map.lookup(TIMEOUT_KEY, MISSING);
    Long timeout = null;
    if (timeoutValue != MISSING && HaraBox.unwrap(timeoutValue) != null) {
      Object numeric = HaraBox.unwrap(timeoutValue);
      if (!(numeric instanceof Number number)
          || number.longValue() < 0
          || number.doubleValue() != (double) number.longValue()) {
        throw new HaraException(
            "std.native.Result/synchronize timeout must be a non-negative integer");
      }
      timeout = number.longValue();
    }

    Object contextValue = map.lookup(CONTEXT_KEY, MISSING);
    IMapType<Object, Object> context =
        contextValue == MISSING ? EMPTY_CONTEXT : contextMap(contextValue);
    return new SynchronizeOptions(timeout, context);
  }

  private static HaraResult timeoutResult(
      Object value, long milliseconds, IMapType<Object, Object> context) {
    IMapType<Object, Object> enriched =
        assocContext(
            context,
            Keyword.create("result", "timeout"),
            milliseconds,
            Keyword.create("result", "cancellation-requested"),
            value instanceof IPromise);

    if (value instanceof IPromise promise) {
      try {
        promise.cancel();
        enriched =
            assocContext(enriched, Keyword.create("result", "cancelled"), Boolean.TRUE);
      } catch (Throwable cancellationError) {
        enriched =
            assocContext(
                enriched,
                Keyword.create("result", "cancelled"),
                Boolean.FALSE,
                Keyword.create("result", "cancellation-error"),
                errorMessage(cancellationError));
      }
    }

    return error(
        resultError("timeout", "Result synchronization timed out", milliseconds),
        enriched);
  }

  private static HaraResult timeoutUnsupportedResult(
      long milliseconds, IMapType<Object, Object> context) {
    return error(
        resultError(
            "timeout-unsupported",
            "Timed synchronization is unsupported for this dereferenceable value",
            milliseconds),
        assocContext(context, Keyword.create("result", "timeout"), milliseconds));
  }

  private static Ex.Info resultError(String code, String message, long milliseconds) {
    return new Ex.Info(
        message,
        hara.lang.data.Map.Standard.from(
            null,
            Keyword.create("code"),
            Keyword.create("result", code),
            Keyword.create("message"),
            message,
            Keyword.create("timeout"),
            milliseconds));
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private static IMapType<Object, Object> assocContext(
      IMapType<Object, Object> context, Object... entries) {
    IMapType result = context;
    for (int index = 0; index < entries.length; index += 2) {
      result = (IMapType) result.assoc(entries[index], entries[index + 1]);
    }
    return (IMapType<Object, Object>) result;
  }

  private record SynchronizeOptions(Long timeout, IMapType<Object, Object> context) {}

  public Keyword status() {
    return Keyword.create(status == Status.SUCCESS ? "success" : "error");
  }

  public Object data() {
    return data;
  }

  public Ex.Info errorValue() {
    return error;
  }

  public IMapType<Object, Object> context() {
    return context;
  }

  @Override
  public Entry<Object, Object> find(Object key) {
    if (!(key instanceof Keyword keyword) || keyword.getNamespace() != null) return null;
    Object value = switch (keyword.getName()) {
      case "status" -> status();
      case "data" -> data();
      case "error" -> errorValue();
      case "context" -> context.count() == 0 ? null : context();
      default -> MISSING;
    };
    return value == MISSING ? null : new hara.lang.data.MapEntry<>(null, keyword, value);
  }

  @Override
  public Iterator<Object> keys() {
    return java.util.List.<Object>of(
            Keyword.create("status"), Keyword.create("data"),
            Keyword.create("error"), Keyword.create("context"))
        .iterator();
  }

  @Override
  public Iterator<Object> vals() {
    return java.util.Arrays.asList(status(), data(), errorValue(), context.count() == 0 ? null : context())
        .iterator();
  }

  public boolean isSuccess() {
    return status == Status.SUCCESS;
  }

  public boolean isError() {
    return status == Status.ERROR;
  }

  @SuppressWarnings("unchecked")
  public boolean isTimeout() {
    if (!isError() || error == null || !(error.getData() instanceof IMapType<?, ?> rawData)) {
      return false;
    }
    IMapType<Object, Object> data = (IMapType<Object, Object>) rawData;
    return Eq.eq(data.lookup(ERROR_CODE_KEY), TIMEOUT_ERROR_CODE);
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  public HaraResult withContext(Object additionalContext) {
    IMapType<Object, Object> additional = contextMap(additionalContext);
    IMapType merged = context;
    for (Object entryValue : additional) {
      Entry entry = (Entry) entryValue;
      merged = (IMapType) merged.assoc(entry.getKey(), entry.getValue());
    }
    return new HaraResult(status, data, error, (IMapType<Object, Object>) merged);
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  IMapType<Object, Object> transportContext() {
    IMapType portable = EMPTY_CONTEXT;
    for (Object entryValue : context) {
      Entry entry = (Entry) entryValue;
      if (!Eq.eq(entry.getKey(), DISPLAY_KEY)) {
        portable = (IMapType) portable.assoc(entry.getKey(), entry.getValue());
      }
    }
    return (IMapType<Object, Object>) portable;
  }

  @Override
  public Object deref() {
    if (isSuccess()) return data;
    throw Ex.Sneaky(error);
  }

  @Override
  public boolean equality(Object other) {
    if (!(HaraBox.unwrap(other) instanceof HaraResult result)) return false;
    return status == result.status
        && Eq.eq(data, result.data)
        && errorEquals(error, result.error);
  }

  @Override
  public long hashCalc(Constant.HashType hashType) {
    long hash = "::RESULT".hashCode();
    hash = hash * 31 + (isSuccess() ? 1 : 2);
    hash = hash * 31 + G.hashCalc(hashType, data);
    hash = hash * 31 + errorHash(error, hashType);
    return hash;
  }

  @Override
  public String display() {
    return "#hara/Result["
        + status().display()
        + " "
        + G.display(data)
        + " "
        + displayError(error)
        + " "
        + G.display(context)
        + "]";
  }

  @Override
  public boolean equals(Object other) {
    return equality(other);
  }

  @Override
  public int hashCode() {
    return Long.hashCode(hashCalc(Constant.HashType.RAPID));
  }

  @Override
  public String toString() {
    return display();
  }

  @SuppressWarnings("unchecked")
  private static IMapType<Object, Object> contextMap(Object value) {
    Object raw = HaraBox.unwrap(value);
    if (!(raw instanceof IMapType<?, ?>)) {
      throw new HaraException("Result context must be a map");
    }
    return (IMapType<Object, Object>) raw;
  }

  private static Ex.Info normalizeError(Object value) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof Ex.Info info) return info;
    if (raw instanceof Throwable throwable && raw instanceof IExInfo info) {
      return new Ex.Info(errorMessage(throwable), info.getData(), throwable.getCause());
    }
    if (raw instanceof Throwable throwable) {
      return new Ex.Info(
          errorMessage(throwable),
          hara.lang.data.Map.Standard.from(
              null,
              Keyword.create("error", "class"),
              throwable.getClass().getName(),
              Keyword.create("error", "message"),
              errorMessage(throwable)),
          throwable.getCause());
    }
    String message = raw instanceof String text ? text : G.display(raw);
    return new Ex.Info(
        message,
        hara.lang.data.Map.Standard.from(
            null, Keyword.create("error", "value"), raw));
  }

  private static String errorMessage(Throwable error) {
    return error.getMessage() == null ? error.getClass().getName() : error.getMessage();
  }

  private static Object errorData(Throwable error) {
    return error instanceof IExInfo info ? info.getData() : null;
  }

  private static boolean errorEquals(Throwable left, Throwable right) {
    if (left == right) return true;
    if (left == null || right == null) return false;
    return left.getClass().equals(right.getClass())
        && Objects.equals(left.getMessage(), right.getMessage())
        && Eq.eq(errorData(left), errorData(right))
        && errorEquals(left.getCause(), right.getCause());
  }

  private static long errorHash(Throwable error, Constant.HashType hashType) {
    if (error == null) return 0;
    long hash = "::RESULT_ERROR".hashCode();
    hash = hash * 31 + "hara/Error".hashCode();
    hash = hash * 31 + Objects.hashCode(error.getMessage());
    hash = hash * 31 + G.hashCalc(hashType, errorData(error));
    hash = hash * 31 + errorHash(error.getCause(), hashType);
    return hash;
  }

  private static String displayError(Throwable error) {
    if (error == null) return "nil";
    return "#error["
        + G.display(errorMessage(error))
        + " "
        + G.display(errorData(error))
        + "]";
  }
}
