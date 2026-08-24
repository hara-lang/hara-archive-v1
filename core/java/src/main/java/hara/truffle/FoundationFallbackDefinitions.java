package hara.truffle;

import hara.lang.data.List;
import hara.lang.data.Symbol;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.ISetType;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.LinkedHashSet;
import java.util.Map;
import java.util.Set;
import java.util.function.Predicate;

/**
 * Names whose complete Truffle semantics require the portable {@code std.foundation} startup.
 *
 * <p>The source is parsed once, on the first Truffle source compilation, but none of its
 * definitions are executed. This keeps demand exact for optimized Java exports that are replaced
 * or completed by a HAL definition. A small set of Java primitives also depends on protocol or
 * reader initialization performed while Foundation executes despite having no same-name HAL
 * definition.
 */
final class FoundationFallbackDefinitions {
  private static final String RESOURCE = "std/foundation.hal";
  private static final Set<String> NAMES = load();
  private static final Set<String> INITIALIZATION_DEPENDENCIES = Set.of();

  private FoundationFallbackDefinitions() {}

  static boolean defines(String name) {
    return NAMES.contains(name);
  }

  static boolean requiresInitialization(String name) {
    return NAMES.contains(name) || INITIALIZATION_DEPENDENCIES.contains(name);
  }

  /**
   * Finds Foundation-defined special symbols and explicit initialization dependencies before the
   * ordinary demand scanner discards language-special operators. This pass is deliberately
   * conservative: a quoted or locally shadowed dependency may load Foundation unnecessarily, but
   * no valid portable behavior is skipped.
   */
  static boolean requiresInitialization(Object[] forms, HaraContext context) {
    return requiresInitialization(forms, context::isSpecialSymbol);
  }

  static boolean requiresInitialization(
      Object[] forms, Predicate<Symbol> specialSymbol) {
    for (Object form : forms) {
      if (requiresInitialization(form, specialSymbol)) return true;
    }
    return false;
  }

  static boolean isInitializationDependency(String name) {
    return INITIALIZATION_DEPENDENCIES.contains(name);
  }

  static Set<String> names() {
    return NAMES;
  }

  private static boolean requiresInitialization(
      Object value, Predicate<Symbol> specialSymbol) {
    if (value instanceof Symbol symbol) {
      if (symbol.getNamespace() != null) return false;
      String name = symbol.getName();
      return INITIALIZATION_DEPENDENCIES.contains(name)
          || (NAMES.contains(name) && specialSymbol.test(symbol));
    }
    if (value instanceof List<?> list) {
      for (int index = 0; index < list.count(); index++) {
        if (requiresInitialization(list.nth(index), specialSymbol)) return true;
      }
      return false;
    }
    if (value instanceof IMapType<?, ?> map) {
      for (Object entryValue : map) {
        if (!(entryValue instanceof Map.Entry<?, ?> entry)) continue;
        if (requiresInitialization(entry.getKey(), specialSymbol)
            || requiresInitialization(entry.getValue(), specialSymbol)) return true;
      }
      return false;
    }
    if (value instanceof ISetType<?> set) {
      for (Object item : set) {
        if (requiresInitialization(item, specialSymbol)) return true;
      }
      return false;
    }
    if (value instanceof ILinearType<?> linear) {
      for (Object item : linear) {
        if (requiresInitialization(item, specialSymbol)) return true;
      }
    }
    return false;
  }

  private static Set<String> load() {
    ClassLoader loader = HaraContext.class.getClassLoader();
    try (InputStream input = loader.getResourceAsStream(RESOURCE)) {
      if (input == null) return Set.of();
      String source = new String(input.readAllBytes(), StandardCharsets.UTF_8);
      Object[] forms = HaraLanguage.readAll(source, RESOURCE);
      LinkedHashSet<String> names = new LinkedHashSet<>();
      for (Object form : forms) collect(form, names);
      return Set.copyOf(names);
    } catch (IOException error) {
      throw new HaraException("Unable to index " + RESOURCE + ": " + error.getMessage());
    }
  }

  private static void collect(Object form, Set<String> names) {
    if (!(form instanceof List<?> list)
        || list.count() == 0
        || !(list.nth(0) instanceof Symbol operator)
        || operator.getNamespace() != null) {
      return;
    }
    String operation = operator.getName();
    if ("do".equals(operation)) {
      for (int index = 1; index < list.count(); index++) collect(list.nth(index), names);
      return;
    }
    if ("declare".equals(operation)) {
      for (int index = 1; index < list.count(); index++) addSymbol(list.nth(index), names);
      return;
    }
    if (!Set.of(
            "def",
            "defn",
            "defn-",
            "defmacro",
            "defstruct",
            "defmutable",
            "defprotocol",
            "defmulti")
        .contains(operation)) {
      return;
    }
    if (list.count() < 2 || !(list.nth(1) instanceof Symbol name)) return;
    names.add(name.getName());
    if ("defstruct".equals(operation) || "defmutable".equals(operation)) {
      names.add("->" + name.getName());
      names.add("map->" + name.getName());
    }
    if ("defprotocol".equals(operation)) {
      for (int index = 2; index < list.count(); index++) {
        Object method = list.nth(index);
        if (method instanceof List<?> declaration && declaration.count() > 0) {
          addSymbol(declaration.nth(0), names);
        }
      }
    }
  }

  private static void addSymbol(Object value, Set<String> names) {
    if (value instanceof Symbol symbol && symbol.getNamespace() == null) {
      names.add(symbol.getName());
    }
  }
}
