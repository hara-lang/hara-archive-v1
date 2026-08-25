package hara.truffle;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;
import java.io.IOException;
import java.net.JarURLConnection;
import java.net.URL;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Enumeration;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.TreeSet;
import java.util.jar.JarEntry;
import java.util.jar.JarFile;
import java.util.stream.Stream;

/** Installs Hara protocol descriptors from the annotated Java interface surface. */
final class HaraProtocolDeclarations {
  private static final String PACKAGE = "hara.lang.protocol";
  private static final String RESOURCE = "hara/lang/protocol";

  private HaraProtocolDeclarations() {}

  static Registry install(HaraContext context) {
    return context.withDeclarationTransaction(() -> installDeclarations(context));
  }

  private static Registry installDeclarations(HaraContext context) {
    Map<String, Class<?>> declarations = discover();
    Map<String, HaraProtocol> installed = new LinkedHashMap<>();
    for (String name : new TreeSet<>(declarations.keySet())) {
      install(context, declarations, installed, name, new HashSet<>());
    }
    return new Registry(
        Collections.unmodifiableMap(new LinkedHashMap<>(installed)),
        Collections.unmodifiableMap(new LinkedHashMap<>(declarations)));
  }

  record Registry(Map<String, HaraProtocol> protocols, Map<String, Class<?>> declarations) {}

  static Map<String, Class<?>> discover() {
    ClassLoader loader = HaraContext.class.getClassLoader();
    Set<String> classNames = new TreeSet<>();
    try {
      Enumeration<URL> resources = loader.getResources(RESOURCE);
      while (resources.hasMoreElements()) collectResource(resources.nextElement(), classNames);
    } catch (IOException error) {
      throw HaraException.withCause("Cannot discover annotated protocol declarations", error);
    }
    if (classNames.isEmpty()) collectClasspath(classNames);

    Map<String, Class<?>> declarations = new LinkedHashMap<>();
    for (String className : classNames) {
      if (!className.startsWith(PACKAGE + ".") || className.contains("$")) continue;
      try {
        Class<?> type = Class.forName(className, false, loader);
        HaraProtocolBinding binding = type.getAnnotation(HaraProtocolBinding.class);
        if (binding == null) continue;
        if (!type.isInterface()) {
          throw new HaraException("Protocol declaration is not an interface: " + className);
        }
        Class<?> previous = declarations.put(binding.name(), type);
        if (previous != null) {
          throw new HaraException(
              "Duplicate annotated protocol " + binding.name() + ": " + previous + " and " + type);
        }
      } catch (ClassNotFoundException error) {
        throw HaraException.withCause("Cannot load protocol declaration " + className, error);
      }
    }
    if (declarations.isEmpty()) {
      throw new HaraException("No annotated Hara protocol declarations were discovered");
    }
    return declarations;
  }

  private static HaraProtocol install(
      HaraContext context,
      Map<String, Class<?>> declarations,
      Map<String, HaraProtocol> installed,
      String name,
      Set<String> visiting) {
    HaraProtocol existing = installed.get(name);
    if (existing != null) return existing;
    if (!visiting.add(name)) throw new HaraException("Cyclic annotated protocol parents at " + name);

    Class<?> type = declarations.get(name);
    if (type == null) throw new HaraException("Missing annotated protocol parent: " + name);
    HaraProtocolBinding binding = type.getAnnotation(HaraProtocolBinding.class);
    String expectedNamespace =
        "std.protocol." + name.toLowerCase(Locale.ROOT);
    if (!expectedNamespace.equals(binding.namespace())) {
      throw new HaraException(
          "Protocol declaration namespace differs from its canonical name: "
              + name
              + " expected "
              + expectedNamespace
              + " but found "
              + binding.namespace());
    }
    List<HaraProtocol> parents = new ArrayList<>();
    for (String parent : binding.parents()) {
      parents.add(install(context, declarations, installed, parent, visiting));
    }

    Map<String, Integer> methods = new LinkedHashMap<>();
    for (java.lang.reflect.Method method : type.getDeclaredMethods()) {
      HaraMethod declaration = method.getAnnotation(HaraMethod.class);
      if (declaration == null) continue;
      if (methods.put(declaration.value(), arity(method, declaration)) != null) {
        throw new HaraException("Duplicate annotated method " + name + "/" + declaration.value());
      }
    }

    HaraProtocol protocol = context.protocol(name);
    if (protocol == null) {
      protocol = context.defineInjectedProtocol(name, methods, parents);
    } else {
      Map<String, Integer> existingMethods = new LinkedHashMap<>();
      protocol.methods().forEach((methodName, method) -> existingMethods.put(methodName, method.arity()));
      if (!existingMethods.equals(methods)) {
        throw new HaraException("Existing protocol descriptor differs from annotations: " + name);
      }
      Set<String> actualParents = new LinkedHashSet<>();
      for (HaraProtocol parent : protocol.parents()) actualParents.add(parent.name());
      Set<String> expectedParents = new LinkedHashSet<>();
      for (HaraProtocol parent : parents) expectedParents.add(parent.name());
      if (!actualParents.equals(expectedParents)) {
        throw new HaraException("Existing protocol parents differ from annotations: " + name);
      }
    }
    installed.put(name, protocol);
    visiting.remove(name);
    return protocol;
  }

  private static int arity(java.lang.reflect.Method method, HaraMethod declaration) {
    if (declaration.arity() != HaraMethod.UNSPECIFIED_ARITY) return declaration.arity();
    return declaration.variadic() ? -1 : method.getParameterCount() + 1;
  }

  private static void collectResource(URL resource, Set<String> names) {
    try {
      if ("file".equals(resource.getProtocol())) {
        collectDirectory(Path.of(resource.toURI()), names);
      } else if ("jar".equals(resource.getProtocol())) {
        JarURLConnection connection = (JarURLConnection) resource.openConnection();
        collectJar(connection.getJarFile(), names);
      }
    } catch (Exception error) {
      throw HaraException.withCause("Cannot scan protocol declaration resource " + resource, error);
    }
  }

  private static void collectClasspath(Set<String> names) {
    String classpath = System.getProperty("java.class.path", "");
    for (String entry : classpath.split(java.io.File.pathSeparator)) {
      Path path = Path.of(entry);
      if (Files.isDirectory(path)) {
        collectDirectory(path.resolve(RESOURCE), names);
      } else if (entry.endsWith(".jar")) {
        try (JarFile jar = new JarFile(path.toFile())) {
          collectJar(jar, names);
        } catch (IOException error) {
          throw HaraException.withCause("Cannot scan classpath entry " + entry, error);
        }
      }
    }
  }

  private static void collectDirectory(Path directory, Set<String> names) {
    if (!Files.isDirectory(directory)) return;
    try (Stream<Path> paths = Files.list(directory)) {
      paths
          .filter(path -> path.getFileName().toString().endsWith(".class"))
          .forEach(
              path ->
                  names.add(
                      PACKAGE
                          + "."
                          + path.getFileName().toString().replaceFirst("\\.class$", "")));
    } catch (IOException error) {
      throw HaraException.withCause("Cannot scan protocol declaration directory " + directory, error);
    }
  }

  private static void collectJar(JarFile jar, Set<String> names) {
    Enumeration<JarEntry> entries = jar.entries();
    while (entries.hasMoreElements()) {
      String name = entries.nextElement().getName();
      if (name.startsWith(RESOURCE + "/") && name.endsWith(".class")) {
        names.add(name.substring(0, name.length() - 6).replace('/', '.'));
      }
    }
  }
}
