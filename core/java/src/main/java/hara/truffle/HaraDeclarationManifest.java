package hara.truffle;

import hara.lang.declaration.HaraAvailability;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraNativeBinding;
import hara.lang.declaration.HaraProtocolBinding;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Map;

/**
 * Stable inspection views derived from the annotated native and protocol declarations.
 *
 * <p>The manifest is deliberately not a runtime catalog or a generated source artifact. It is a
 * normalized comparison surface for conformance tests and diagnostics.
 */
final class HaraDeclarationManifest {
  private HaraDeclarationManifest() {}

  static List<String> nativeManifest() {
    List<String> manifest = new ArrayList<>();
    for (HaraNativeBinding binding : HaraNativeDeclarations.bindings()) {
      List<String> methods =
          Arrays.stream(binding.methods())
              .map(method -> "std.native." + binding.name() + "/" + method)
              .sorted()
              .toList();
      manifest.add(
          String.join(
              "|",
              "native",
              "std.native." + binding.name(),
              availability(binding.availability()),
              binding.capability(),
              "annotation",
              String.join(",", methods)));
    }
    return manifest.stream().sorted().toList();
  }

  static List<String> protocolManifest() {
    return protocolManifest(HaraProtocolDeclarations.discover());
  }

  static List<String> protocolManifest(Map<String, Class<?>> declarations) {
    List<String> manifest = new ArrayList<>();
    for (Class<?> type : declarations.values()) {
      HaraProtocolBinding binding = type.getAnnotation(HaraProtocolBinding.class);
      if (binding == null) {
        throw new HaraException("Protocol declaration is not annotated: " + type.getName());
      }
      List<String> parents = Arrays.stream(binding.parents()).sorted().toList();
      List<String> methods =
          Arrays.stream(type.getDeclaredMethods())
              .filter(method -> !Modifier.isStatic(method.getModifiers()))
              .map(
                  method -> {
                    HaraMethod declaration = method.getAnnotation(HaraMethod.class);
                    if (declaration == null) return null;
                    return binding.namespace()
                        + "."
                        + binding.name()
                        + "/"
                        + declaration.value()
                        + ":"
                        + methodArity(method, declaration);
                  })
              .filter(java.util.Objects::nonNull)
              .sorted()
              .toList();
      manifest.add(
          String.join(
              "|",
              "protocol",
              binding.namespace() + "." + binding.name(),
              binding.name(),
              availability(binding.availability()),
              binding.capability(),
              "annotation",
              String.join(",", parents),
              String.join(",", methods)));
    }
    return manifest.stream().sorted().toList();
  }

  private static int methodArity(Method method, HaraMethod declaration) {
    if (declaration.arity() != HaraMethod.UNSPECIFIED_ARITY) return declaration.arity();
    return declaration.variadic() ? -1 : method.getParameterCount() + 1;
  }

  private static String availability(HaraAvailability availability) {
    return switch (availability) {
      case PORTABLE -> "portable";
      case CAPABILITY_GATED -> "capability-gated";
      case INVENTORY_ONLY -> "inventory-only";
    };
  }
}
