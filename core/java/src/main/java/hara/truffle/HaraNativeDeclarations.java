package hara.truffle;

import hara.lang.declaration.HaraNativeBinding;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

/** Runtime view of the annotated native type surface. */
final class HaraNativeDeclarations {
  private static final Map<String, HaraNativeBinding> BINDINGS = bindingsByName();
  static final Map<String, List<String>> METHODS = methodInventory();

  private HaraNativeDeclarations() {}

  static List<HaraNativeBinding> bindings() {
    return List.copyOf(BINDINGS.values());
  }

  static HaraNativeBinding binding(String name) {
    HaraNativeBinding binding = BINDINGS.get(name);
    if (binding == null) throw new HaraException("Missing annotated native type: " + name);
    return binding;
  }

  static String namespace(String name) {
    HaraNativeBinding binding = binding(name);
    return binding.namespace() + "." + binding.name();
  }

  static List<String> methods(String name) {
    return List.of(binding(name).methods());
  }

  private static Map<String, List<String>> methodInventory() {
    Map<String, List<String>> methods = new LinkedHashMap<>();
    for (HaraNativeBinding binding : BINDINGS.values()) {
      methods.put(binding.name(), List.of(binding.methods()));
    }
    return Map.copyOf(methods);
  }

  private static Map<String, HaraNativeBinding> bindingsByName() {
    Map<String, HaraNativeBinding> bindings = new LinkedHashMap<>();
    for (HaraNativeBinding binding :
        HaraBuiltinCatalog.class.getAnnotationsByType(HaraNativeBinding.class)) {
      if (!"std.native".equals(binding.namespace())) {
        throw new HaraException("Native binding must use std.native: " + binding.name());
      }
      if (bindings.put(binding.name(), binding) != null) {
        throw new HaraException("Duplicate annotated native type: " + binding.name());
      }
      if (binding.methods().length == 0) {
        throw new HaraException("Native binding has no methods: " + binding.name());
      }
      if (binding.availability() == hara.lang.declaration.HaraAvailability.CAPABILITY_GATED
          && binding.capability().isBlank()) {
        throw new HaraException("Capability-gated native binding has no capability: " + binding.name());
      }
      if (binding.availability() != hara.lang.declaration.HaraAvailability.CAPABILITY_GATED
          && !binding.capability().isBlank()) {
        throw new HaraException("Portable native binding declares a capability: " + binding.name());
      }
      Set<String> methods = new java.util.HashSet<>();
      for (String method : binding.methods()) {
        if (!methods.add(method)) {
          throw new HaraException("Duplicate annotated native method: " + binding.name() + "/" + method);
        }
      }
    }
    return Map.copyOf(bindings);
  }
}
