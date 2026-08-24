package hara.truffle;

import hara.lang.declaration.HaraNativeBinding;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/** Runtime view of the annotated native type surface. */
final class HaraNativeDeclarations {
  private static final Map<String, HaraNativeBinding> BINDINGS = bindingsByName();

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
    List<String> methods = HaraBuiltinCatalog.NATIVE_METHODS.get(name);
    if (methods == null) throw new HaraException("Native annotation has no catalog entry: " + name);
    return methods;
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
      if (!HaraBuiltinCatalog.NATIVE_METHODS.containsKey(binding.name())) {
        throw new HaraException("Native annotation has no catalog entry: " + binding.name());
      }
    }
    if (!bindings.keySet().equals(HaraBuiltinCatalog.NATIVE_METHODS.keySet())) {
      throw new HaraException("Native type catalog is not closed by annotations");
    }
    return Map.copyOf(bindings);
  }
}
