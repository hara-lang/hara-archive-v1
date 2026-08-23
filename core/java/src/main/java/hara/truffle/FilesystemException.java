package hara.truffle;

import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.Map;

/** Stable provider-neutral failure carried by an exceptional CompletionStage. */
public final class FilesystemException extends RuntimeException {
  private static final long serialVersionUID = 1L;
  private final String code;
  private final String provider;
  private final String operation;
  private final String path;
  private final String target;
  private final String providerCode;
  private final boolean retryable;

  public FilesystemException(
      String code,
      String message,
      String provider,
      String operation,
      String path,
      String target,
      String providerCode,
      boolean retryable,
      Throwable cause) {
    super(message, cause);
    this.code = require(code, "filesystem error code");
    this.provider = provider;
    this.operation = operation;
    this.path = path;
    this.target = target;
    this.providerCode = providerCode;
    this.retryable = retryable;
  }

  public String code() {
    return code;
  }

  public String provider() {
    return provider;
  }

  public String operation() {
    return operation;
  }

  public String path() {
    return path;
  }

  public String target() {
    return target;
  }

  public String providerCode() {
    return providerCode;
  }

  public boolean retryable() {
    return retryable;
  }

  public Map<String, Object> data() {
    LinkedHashMap<String, Object> values = new LinkedHashMap<>();
    values.put("ex/code", "file/" + code);
    values.put("file/provider", provider);
    values.put("file/operation", operation);
    values.put("file/path", path);
    values.put("file/target", target);
    values.put("file/provider-code", providerCode);
    values.put("file/retryable?", retryable);
    return Collections.unmodifiableMap(values);
  }

  public static FilesystemException cancelled(
      String provider, String operation, String path, String target) {
    return new FilesystemException(
        "cancelled",
        "filesystem operation cancelled",
        provider,
        operation,
        path,
        target,
        null,
        false,
        null);
  }

  public static FilesystemException timeout(
      String provider, String operation, String path, String target) {
    return new FilesystemException(
        "timeout",
        "filesystem operation timed out",
        provider,
        operation,
        path,
        target,
        null,
        true,
        null);
  }

  public static FilesystemException providerClosed(
      String provider, String operation, String path, String target) {
    return new FilesystemException(
        "provider-closed",
        "filesystem provider is closed",
        provider,
        operation,
        path,
        target,
        null,
        false,
        null);
  }

  public static FilesystemException unsupportedRevision(
      String provider, String operation, String path, String target) {
    return new FilesystemException(
        "unsupported",
        "filesystem provider does not support revision checks",
        provider,
        operation,
        path,
        target,
        null,
        false,
        null);
  }

  public static FilesystemException fromLegacy(
      String provider, String operation, String path, String target, Throwable error) {
    Throwable cause = unwrap(error);
    String code = HaraFileProvider.code(cause);
    return new FilesystemException(
        code,
        cause.getMessage() == null ? "filesystem operation failed" : cause.getMessage(),
        provider,
        operation,
        path,
        target,
        code,
        "io".equals(code),
        cause);
  }

  private static Throwable unwrap(Throwable error) {
    Throwable current = error;
    while ((current instanceof java.util.concurrent.CompletionException
            || current instanceof java.util.concurrent.ExecutionException)
        && current.getCause() != null) {
      current = current.getCause();
    }
    return current;
  }

  private static String require(String value, String label) {
    if (value == null || value.isBlank()) {
      throw new IllegalArgumentException(label + " is required");
    }
    return value;
  }
}
