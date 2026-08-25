package hara.truffle;

import hara.lang.base.Ex;
import hara.lang.data.Keyword;

/** Context-local capability policy and the native denial contract. */
final class HaraNativeCapabilityBoundary {
  private final boolean kernelGranted;
  private final boolean sandboxGranted;
  private final boolean fileGranted;
  private final boolean networkGranted;
  private final boolean nativeRuntimeGranted;
  private final boolean hostCallGranted;

  HaraNativeCapabilityBoundary(
      boolean kernelGranted,
      boolean sandboxGranted,
      boolean fileGranted,
      boolean networkGranted,
      boolean nativeRuntimeGranted,
      boolean hostCallGranted) {
    this.kernelGranted = kernelGranted;
    this.sandboxGranted = sandboxGranted;
    this.fileGranted = fileGranted;
    this.networkGranted = networkGranted;
    this.nativeRuntimeGranted = nativeRuntimeGranted;
    this.hostCallGranted = hostCallGranted;
  }

  boolean granted(String capability) {
    return switch (capability) {
      case "kernel" -> kernelGranted;
      case "sandbox" -> sandboxGranted;
      case "file" -> fileGranted;
      case "network" -> networkGranted;
      case "native-runtime" -> nativeRuntimeGranted;
      case "host-call" -> hostCallGranted;
      default -> false;
    };
  }

  void require(String nativeType, String method, String capability) {
    if (!granted(capability)) {
      throw denied(nativeType, method, capability);
    }
  }

  static Ex.Info denied(String nativeType, String method, String capability) {
    return new Ex.Info(
        "std.native." + nativeType + "/" + method + " requires capability :" + capability,
        hara.lang.data.Map.Standard.from(
            null,
            Keyword.create("ex", "code"),
            Keyword.create("native", "capability-denied"),
            Keyword.create("ex", "class"),
            Keyword.create("ex.class", "host"),
            Keyword.create("native", "type"),
            "std.native." + nativeType,
            Keyword.create("native", "method"),
            method,
            Keyword.create("native", "capability"),
            Keyword.create(capability)));
  }

  static String method(String operation) {
    int separator = operation.lastIndexOf('/');
    String method = separator < 0 ? operation : operation.substring(separator + 1);
    return method.startsWith("process-") ? method.substring("process-".length()) : method;
  }
}
