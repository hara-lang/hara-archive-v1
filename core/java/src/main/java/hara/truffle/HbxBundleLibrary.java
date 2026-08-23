package hara.truffle;

import hara.truffle.bytecode.HbxBundleCodec;
import hara.truffle.bytecode.HbcCodec;
import hara.truffle.bytecode.HbcFormatException;
import hara.truffle.bytecode.HbcProgram;
import java.io.IOException;
import java.io.InputStream;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

/** Immutable namespace index over the shared Rust-produced HBX0 standard-library bundle. */
final class HbxBundleLibrary {
  static final String RESOURCE = "std.foundation.hbx";
  private static final Set<String> SOURCE_OVERRIDES =
      Set.of(
          "std.foundation",
          "code.test",
          "code.test.artifact",
          "code.test.base.context",
          "code.test.base.executive",
          "std.foundation.pretty");
  private static final Set<String> REMOVED_MODULES =
      Set.of(
          "std.pretty",
          "std.foundation.pretty.engine",
          "code.test.selector",
          "std.work",
          "std.work.command",
          "std.work.command.cli",
          "std.work.command.report",
          "std.work.command.result",
          "std.work.command.selector",
          "std.work.command.task",
          "std.work.recipe",
          "std.work.report");

  record Module(String namespace, HbxBundleCodec.Module descriptor, HbcProgram program) {}

  private final Map<String, Module> modules;

  HbxBundleLibrary(ClassLoader loader) {
    this.modules = load(loader);
  }

  boolean available() {
    return !modules.isEmpty();
  }

  boolean provides(String namespace) {
    return modules.containsKey(namespace);
  }

  Module module(String namespace) {
    return modules.get(namespace);
  }

  Iterable<String> namespaces() {
    return modules.keySet();
  }

  List<Module> eagerModules() {
    ArrayList<Module> eager = new ArrayList<>();
    for (Module module : modules.values()) {
      if (module.descriptor().eager()) eager.add(module);
    }
    return List.copyOf(eager);
  }

  private static Map<String, Module> load(ClassLoader loader) {
    try (InputStream input = loader.getResourceAsStream(RESOURCE)) {
      if (input == null) return Map.of();
      List<HbxBundleCodec.Module> descriptors;
      try {
        descriptors = HbxBundleCodec.decode(input.readAllBytes());
      } catch (HbcFormatException error) {
        // The bundle is an optional acceleration layer; source-backed libraries remain usable.
        return Map.of();
      }
      LinkedHashMap<String, Module> indexed = new LinkedHashMap<>();
      for (HbxBundleCodec.Module descriptor : descriptors) {
        if (REMOVED_MODULES.contains(descriptor.resource())
            || descriptor.resource().equals("std.work")
            || descriptor.resource().startsWith("std.work.")) continue;
        // Hara deliberately has no :refer-clojure mode. Legacy generated
        // modules and explicitly source-owned overlays are resolved from the
        // current HAL resources until the tracked bundle can be regenerated.
        if (descriptor.namespaceForm().contains(":refer-clojure")
            || SOURCE_OVERRIDES.contains(descriptor.resource())) {
          continue;
        }
        HbcProgram program = HbcCodec.decode(descriptor.artifact());
        String namespace =
            program.namespace() == null ? descriptor.resource() : program.namespace();
        if (namespace == null || namespace.isBlank()) {
          throw new HbcFormatException("HBX0 module has no namespace: " + descriptor.resource());
        }
        if (indexed.put(namespace, new Module(namespace, descriptor, program)) != null) {
          throw new HbcFormatException("HBX0 contains duplicate namespace: " + namespace);
        }
      }
      return Map.copyOf(indexed);
    } catch (HbcFormatException error) {
      // The bundle is an optional acceleration layer; source-backed libraries remain
      // authoritative when an artifact was produced by an incompatible HBC version.
      return Map.of();
    } catch (IOException error) {
      throw new HaraException("Unable to read " + RESOURCE + ": " + error.getMessage());
    }
  }
}
