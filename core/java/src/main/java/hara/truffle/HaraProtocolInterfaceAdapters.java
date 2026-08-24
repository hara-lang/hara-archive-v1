package hara.truffle;

import hara.lang.declaration.HaraMethod;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Comparator;
import java.util.List;
import java.util.Map;

/** Installs ordinary Java-interface protocol implementations from their annotations. */
final class HaraProtocolInterfaceAdapters {
  private HaraProtocolInterfaceAdapters() {}

  static void install(HaraProtocolDeclarations.Registry registry) {
    for (Map.Entry<String, Class<?>> declaration : registry.declarations().entrySet()) {
      HaraProtocol protocol = registry.protocols().get(declaration.getKey());
      if (protocol == null) {
        throw new HaraException("Missing injected protocol: " + declaration.getKey());
      }
      install(protocol, declaration.getValue());
    }
  }

  static void install(HaraProtocol protocol, Class<?> owner) {
    for (Method method : owner.getDeclaredMethods()) {
      HaraMethod binding = method.getAnnotation(HaraMethod.class);
      if (binding == null || Modifier.isStatic(method.getModifiers())) continue;
      method.trySetAccessible();
      protocol.extend(owner, binding.value(), invoker(owner, method, binding));
    }
  }

  private static HaraProtocolInvoker invoker(Class<?> owner, Method annotated, HaraMethod binding) {
    List<Method> candidates =
        Arrays.stream(owner.getDeclaredMethods())
            .filter(method -> !Modifier.isStatic(method.getModifiers()))
            .filter(method -> method.getName().equals(annotated.getName()))
            .sorted(Comparator.comparingInt(Method::getParameterCount))
            .toList();
    return new HaraProtocolInvoker() {
      @Override
      public Object invoke(Object receiver, Object[] arguments) {
        Method method = select(candidates, receiver, arguments, binding);
        Object[] invocationArguments = invocationArguments(method, arguments);
        try {
          Object result = method.invoke(receiver, invocationArguments);
          return method.getReturnType() == void.class ? receiver : result;
        } catch (IllegalAccessException error) {
          throw HaraException.withCause(
              "Cannot invoke annotated protocol method " + owner.getSimpleName() + "/" + binding.value(),
              error);
        } catch (InvocationTargetException error) {
          Throwable cause = error.getCause();
          if (cause instanceof RuntimeException runtime) throw runtime;
          if (cause instanceof Error fatal) throw fatal;
          throw HaraException.withCause(
              "Annotated protocol method failed " + owner.getSimpleName() + "/" + binding.value(),
              cause);
        }
      }

      @Override
      public int arity() {
        if (binding.variadic()) return -1;
        if (binding.arity() != HaraMethod.UNSPECIFIED_ARITY) return binding.arity();
        return annotated.getParameterCount() + 1;
      }
    };
  }

  private static Method select(
      List<Method> candidates, Object receiver, Object[] arguments, HaraMethod binding) {
    List<Method> matches = new ArrayList<>();
    for (Method candidate : candidates) {
      int argumentCount = arguments.length;
      int fixedCount = candidate.getParameterCount() - (candidate.isVarArgs() ? 1 : 0);
      if (candidate.isVarArgs() ? argumentCount >= fixedCount : argumentCount == candidate.getParameterCount()) {
        matches.add(candidate);
      }
    }
    if (matches.isEmpty()) {
      throw new HaraException(
          "No Java overload for "
              + receiver.getClass().getName()
              + "/"
              + binding.value()
              + " with "
              + arguments.length
              + " arguments");
    }
    return matches.get(0);
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
}
