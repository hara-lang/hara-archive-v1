package hara.truffle;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;
import hara.lang.declaration.HaraProtocolExtension;
import hara.lang.declaration.HaraProtocolTarget;
import java.io.IOException;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.net.JarURLConnection;
import java.net.URL;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.Comparator;
import java.util.Enumeration;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeSet;
import java.util.jar.JarEntry;
import java.util.jar.JarFile;
import java.util.stream.Stream;

/** Installs the complete built-in protocol surface from annotated declarations. */
final class HaraProtocolRuntime {
  private static final String EXTENSION_PACKAGE = "hara.truffle";
  private static final String EXTENSION_RESOURCE = "hara/truffle";

  private HaraProtocolRuntime() {}

  static void install(HaraContext context, HaraProtocolDeclarations.Registry registry) {
    installInterfaces(registry);
    installExtensions(context, registry, true);
  }

  /** Installs the same annotations into one protocol for focused Java dispatch tests. */
  static void installForTest(HaraProtocol protocol) {
    try {
      Class<?> declaration =
          Class.forName("hara.lang.protocol." + protocol.name(), false, HaraContext.class.getClassLoader());
      HaraProtocolDeclarations.Registry registry =
          new HaraProtocolDeclarations.Registry(
              Map.of(protocol.name(), protocol), Map.of(protocol.name(), declaration));
      installInterfaces(registry);
      installExtensions(null, registry, false);
    } catch (ClassNotFoundException error) {
      throw HaraException.withCause("Missing annotated test protocol: " + protocol.name(), error);
    }
  }

  private static void installInterfaces(HaraProtocolDeclarations.Registry registry) {
    for (Map.Entry<String, Class<?>> declaration : registry.declarations().entrySet()) {
      HaraProtocol protocol = registry.protocols().get(declaration.getKey());
      if (protocol == null) {
        throw new HaraException("Missing injected protocol: " + declaration.getKey());
      }
      for (Method method : declaration.getValue().getDeclaredMethods()) {
        HaraMethod binding = method.getAnnotation(HaraMethod.class);
        if (Modifier.isStatic(method.getModifiers())) continue;
        String methodName = binding == null ? method.getName() : binding.value();
        HaraProtocol.HaraProtocolMethod protocolMethod = protocol.method(methodName);
        if (protocolMethod == null) continue;
        method.trySetAccessible();
        protocol.extend(
            declaration.getValue(),
            methodName,
            interfaceInvoker(
                declaration.getValue(),
                method,
                methodName,
                protocolMethod.arity()));
      }
    }
  }

  private static HaraProtocolInvoker interfaceInvoker(
      Class<?> owner, Method annotated, String methodName, int protocolArity) {
    List<Method> candidates =
        Arrays.stream(owner.getDeclaredMethods())
            .filter(method -> !Modifier.isStatic(method.getModifiers()))
            .filter(method -> method.getName().equals(annotated.getName()))
            .sorted(Comparator.comparingInt(Method::getParameterCount))
            .toList();
    return new HaraProtocolInvoker() {
      @Override
      public Object invoke(Object receiver, Object[] arguments) {
        Method method = select(candidates, receiver, arguments, methodName);
        Object[] invocationArguments = invocationArguments(method, arguments);
        try {
          Object result = method.invoke(receiver, invocationArguments);
          return method.getReturnType() == void.class ? receiver : result;
        } catch (IllegalAccessException error) {
          throw HaraException.withCause(
              "Cannot invoke Java protocol method " + owner.getSimpleName() + "/" + methodName,
              error);
        } catch (InvocationTargetException error) {
          Throwable cause = error.getCause();
          if (cause instanceof RuntimeException runtime) throw runtime;
          if (cause instanceof Error fatal) throw fatal;
          throw HaraException.withCause(
              "Java protocol method failed " + owner.getSimpleName() + "/" + methodName,
              cause);
        }
      }

      @Override
      public int arity() {
        return protocolArity;
      }
    };
  }

  private static Method select(
      List<Method> candidates,
      Object receiver,
      Object[] arguments,
      String methodName) {
    for (Method candidate : candidates) {
      int argumentCount = arguments.length;
      int fixedCount = candidate.getParameterCount() - (candidate.isVarArgs() ? 1 : 0);
      if (candidate.isVarArgs() ? argumentCount >= fixedCount : argumentCount == candidate.getParameterCount()) {
        return candidate;
      }
    }
    throw new HaraException(
        "No Java overload for "
            + receiver.getClass().getName()
            + "/"
            + methodName
            + " with "
            + arguments.length
            + " arguments");
  }

  private static Object[] invocationArguments(Method method, Object[] arguments) {
    if (!method.isVarArgs()) return arguments;
    int fixedCount = method.getParameterCount() - 1;
    if (arguments.length < fixedCount) {
      throw new HaraException("Not enough arguments for " + method.getName());
    }
    Object[] packed = new Object[method.getParameterCount()];
    System.arraycopy(arguments, 0, packed, 0, fixedCount);
    packed[fixedCount] = Arrays.copyOfRange(arguments, fixedCount, arguments.length);
    return packed;
  }

  private static void installExtensions(
      HaraContext context, HaraProtocolDeclarations.Registry registry, boolean requireAll) {
    Set<String> explicitKeys = new HashSet<>();
    for (String className : discoverExtensionClasses()) {
      try {
        Class<?> owner = Class.forName(className, false, HaraContext.class.getClassLoader());
        for (Method method : owner.getDeclaredMethods()) {
          HaraProtocolExtension[] bindings =
              method.getAnnotationsByType(HaraProtocolExtension.class);
          if (bindings.length == 0) continue;
          validateCallback(owner, method);
          method.trySetAccessible();
          for (HaraProtocolExtension binding : bindings) {
            installExtension(context, registry, explicitKeys, method, binding, requireAll);
          }
        }
      } catch (ClassNotFoundException error) {
        throw HaraException.withCause("Cannot load protocol extension " + className, error);
      }
    }
  }

  private static void validateCallback(Class<?> owner, Method method) {
    if (!Modifier.isStatic(method.getModifiers())) {
      throw new HaraException("Protocol extension must be static: " + owner.getName() + "." + method.getName());
    }
    Class<?>[] parameters = method.getParameterTypes();
    boolean valid =
        Arrays.equals(parameters, new Class<?>[] {Object.class, Object[].class})
            || Arrays.equals(parameters, new Class<?>[] {HaraContext.class, Object.class, Object[].class});
    if (!valid || method.getReturnType() == void.class) {
      throw new HaraException(
          "Protocol extension must use (Object,Object[]) or (HaraContext,Object,Object[]) and return a value: "
              + owner.getName()
              + "."
              + method.getName());
    }
  }

  private static void installExtension(
      HaraContext context,
      HaraProtocolDeclarations.Registry registry,
      Set<String> explicitKeys,
      Method callback,
      HaraProtocolExtension binding,
      boolean requireAll) {
    HaraProtocolBinding protocolBinding =
        binding.protocol().getAnnotation(HaraProtocolBinding.class);
    if (protocolBinding == null) {
      throw new HaraException("Extension protocol is not annotated: " + binding.protocol().getName());
    }
    HaraProtocol protocol = registry.protocols().get(protocolBinding.name());
    if (protocol == null) {
      if (!requireAll) return;
      throw new HaraException("Missing injected protocol for extension: " + protocolBinding.name());
    }
    HaraProtocol.HaraProtocolMethod protocolMethod = protocol.method(binding.method());
    if (protocolMethod == null) {
      throw new HaraException(
          "Unknown protocol extension method " + protocolBinding.name() + "/" + binding.method());
    }
    validateTarget(binding, callback);
    String key = extensionKey(protocolBinding.name(), binding);
    if (!explicitKeys.add(key)) {
      throw new HaraException("Duplicate annotated protocol extension: " + key);
    }
    HaraProtocolInvoker invoker = callbackInvoker(context, callback, protocolMethod.arity());
    switch (binding.target()) {
      case JAVA_CLASS ->
          extendJava(protocol, binding, invoker);
      case NIL -> {
        if (binding.intrinsic()) protocol.extendNilIntrinsic(binding.method(), invoker);
        else protocol.extendNil(binding.method(), invoker);
      }
      case FOREIGN -> protocol.extendForeign(binding.method(), invoker);
      case DEFAULT -> protocol.extendDefault(binding.method(), invoker);
      case BOOLEAN, NUMBER, CHARACTER, STRING ->
          protocol.extend(primitive(binding.target()), binding.method(), invoker);
    }
  }

  private static void validateTarget(HaraProtocolExtension binding, Method callback) {
    boolean javaClass = binding.target() == HaraProtocolTarget.JAVA_CLASS;
    boolean hasReceiver = binding.receiver() != Void.class;
    if (javaClass != hasReceiver) {
      throw new HaraException(
          "JAVA_CLASS extensions require receiver and category extensions forbid it: "
              + callback.getDeclaringClass().getName()
              + "."
              + callback.getName());
    }
  }

  private static void extendJava(
      HaraProtocol protocol, HaraProtocolExtension binding, HaraProtocolInvoker invoker) {
    if (binding.intrinsic()) {
      protocol.extendIntrinsic(binding.receiver(), binding.method(), invoker);
    } else {
      protocol.extend(binding.receiver(), binding.method(), invoker);
    }
  }

  private static HaraDispatchKey.PrimitiveCategory primitive(HaraProtocolTarget target) {
    return switch (target) {
      case BOOLEAN -> HaraDispatchKey.PrimitiveCategory.BOOLEAN;
      case NUMBER -> HaraDispatchKey.PrimitiveCategory.NUMBER;
      case CHARACTER -> HaraDispatchKey.PrimitiveCategory.CHARACTER;
      case STRING -> HaraDispatchKey.PrimitiveCategory.STRING;
      default -> throw new AssertionError(target);
    };
  }

  private static String extensionKey(String protocol, HaraProtocolExtension binding) {
    return protocol
        + "/"
        + binding.method()
        + ":"
        + binding.target()
        + ":"
        + binding.receiver().getName();
  }

  private static HaraProtocolInvoker callbackInvoker(
      HaraContext context, Method callback, int arity) {
    boolean acceptsContext = callback.getParameterCount() == 3;
    return new HaraProtocolInvoker() {
      @Override
      public Object invoke(Object receiver, Object[] arguments) {
        try {
          return acceptsContext
              ? callback.invoke(null, context, receiver, arguments)
              : callback.invoke(null, receiver, arguments);
        } catch (IllegalAccessException error) {
          throw HaraException.withCause("Cannot invoke protocol extension " + callback, error);
        } catch (InvocationTargetException error) {
          Throwable cause = error.getCause();
          if (cause instanceof RuntimeException runtime) throw runtime;
          if (cause instanceof Error fatal) throw fatal;
          throw HaraException.withCause("Protocol extension failed " + callback, cause);
        }
      }

      @Override
      public int arity() {
        return arity;
      }
    };
  }

  private static Set<String> discoverExtensionClasses() {
    Set<String> classNames = new TreeSet<>();
    ClassLoader loader = HaraContext.class.getClassLoader();
    try {
      Enumeration<URL> resources = loader.getResources(EXTENSION_RESOURCE);
      while (resources.hasMoreElements()) collectResource(resources.nextElement(), classNames);
    } catch (IOException error) {
      throw HaraException.withCause("Cannot discover annotated protocol extensions", error);
    }
    if (classNames.isEmpty()) collectClasspath(classNames);
    return classNames;
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
      throw HaraException.withCause("Cannot scan protocol extension resource " + resource, error);
    }
  }

  private static void collectClasspath(Set<String> names) {
    String classpath = System.getProperty("java.class.path", "");
    for (String entry : classpath.split(java.io.File.pathSeparator)) {
      Path path = Path.of(entry);
      if (Files.isDirectory(path)) {
        collectDirectory(path.resolve(EXTENSION_RESOURCE), names);
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
          .filter(path -> !path.getFileName().toString().contains("$"))
          .forEach(
              path ->
                  names.add(
                      EXTENSION_PACKAGE
                          + "."
                          + path.getFileName().toString().replaceFirst("\\.class$", "")));
    } catch (IOException error) {
      throw HaraException.withCause("Cannot scan protocol extension directory " + directory, error);
    }
  }

  private static void collectJar(JarFile jar, Set<String> names) {
    Enumeration<JarEntry> entries = jar.entries();
    while (entries.hasMoreElements()) {
      String name = entries.nextElement().getName();
      if (name.startsWith(EXTENSION_RESOURCE + "/")
          && name.endsWith(".class")
          && !name.contains("$")) {
        names.add(name.substring(0, name.length() - 6).replace('/', '.'));
      }
    }
  }
}
