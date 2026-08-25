package hara.truffle;

import com.oracle.truffle.api.CompilerDirectives.TruffleBoundary;
import com.oracle.truffle.api.interop.InteropLibrary;
import com.oracle.truffle.api.interop.TruffleObject;
import com.oracle.truffle.api.library.ExportLibrary;
import com.oracle.truffle.api.library.ExportMessage;
import hara.lang.base.Ex;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.List;

@ExportLibrary(InteropLibrary.class)
public final class HaraProtocol implements TruffleObject {
  private final String name;
  private final Map<String, HaraProtocolMethod> methods;
  private final List<HaraProtocol> parents;
  private final HaraDispatchRegistry implementations = new HaraDispatchRegistry();

  public HaraProtocol(String name, Map<String, Integer> methodArities) {
    this(name, methodArities, List.of());
  }

  public HaraProtocol(
      String name, Map<String, Integer> methodArities, List<HaraProtocol> parents) {
    this.name = name;
    this.parents = List.copyOf(parents);
    Map<String, HaraProtocolMethod> descriptors = new LinkedHashMap<>();
    for (Map.Entry<String, Integer> entry : methodArities.entrySet()) {
      descriptors.put(
          entry.getKey(), new HaraProtocolMethod(this, entry.getKey(), entry.getValue()));
    }
    methods = Collections.unmodifiableMap(descriptors);
  }

  public String name() {
    return name;
  }

  public HaraProtocolMethod method(String methodName) {
    return methods.get(methodName);
  }

  public Map<String, HaraProtocolMethod> methods() {
    return methods;
  }

  public List<HaraProtocol> parents() {
    return parents;
  }

  public com.oracle.truffle.api.Assumption implementationsStable() {
    return implementations.stable();
  }

  public void extend(HaraType type, String methodName, HaraFunction function) {
    extend(HaraDispatchKey.haraType(type), methodName, functionInvoker(function), function);
  }

  public void extend(HaraType type, String methodName, HaraProtocolInvoker invoker) {
    extend(HaraDispatchKey.haraType(type), methodName, invoker, null);
  }

  public void extend(Class<?> type, String methodName, HaraProtocolInvoker invoker) {
    extend(HaraDispatchKey.javaClass(type), methodName, invoker, null);
  }

  public void extend(
      HaraDispatchKey.PrimitiveCategory category, String methodName, HaraProtocolInvoker invoker) {
    extend(HaraDispatchKey.primitive(category), methodName, invoker, null);
  }

  public void extendNil(String methodName, HaraProtocolInvoker invoker) {
    extend(HaraDispatchKey.nil(), methodName, invoker, null);
  }

  /** Registers a built-in invoker that specialized nodes are allowed to inline. */
  void extendIntrinsic(Class<?> type, String methodName, HaraProtocolInvoker invoker) {
    extend(HaraDispatchKey.javaClass(type), methodName, invoker, null, true);
  }

  /** Registers a built-in nil invoker that specialized nodes are allowed to inline. */
  void extendNilIntrinsic(String methodName, HaraProtocolInvoker invoker) {
    extend(HaraDispatchKey.nil(), methodName, invoker, null, true);
  }

  public void extendForeign(String methodName, HaraProtocolInvoker invoker) {
    extend(HaraDispatchKey.foreign(), methodName, invoker, null);
  }

  public void extendDefault(String methodName, HaraProtocolInvoker invoker) {
    extend(HaraDispatchKey.defaultKey(), methodName, invoker, null);
  }

  @TruffleBoundary
  private void extend(
      HaraDispatchKey key, String methodName, HaraProtocolInvoker invoker, HaraFunction function) {
    extend(key, methodName, invoker, function, false);
  }

  @TruffleBoundary
  private void extend(
      HaraDispatchKey key,
      String methodName,
      HaraProtocolInvoker invoker,
      HaraFunction function,
      boolean intrinsic) {
    HaraProtocolMethod method = method(methodName);
    if (method == null) {
      throw new HaraException("Unknown method " + name + "/" + methodName);
    }
    if (!method.acceptsCallArity(invoker.arity())) {
      throw new HaraException(
          name
              + "/"
              + methodName
              + " expects "
              + method.arity()
              + " arguments, received "
              + invoker.arity());
    }
    implementations.register(
        methodName, key, new HaraProtocolImplementation(invoker, function, intrinsic));
  }

  public HaraProtocolImplementation implementation(Object receiver, String methodName) {
    if (method(methodName) == null) {
      return null;
    }
    return implementations.resolve(methodName, receiver);
  }

  /** Returns true when receiver implements every method in this protocol. */
  @TruffleBoundary
  public boolean satisfies(Object receiver) {
    for (HaraProtocol parent : parents) {
      if (!parent.satisfies(receiver)) return false;
    }
    if (methods.isEmpty()) {
      if (name.endsWith(".IOFn") || name.endsWith("/IOFn")) {
        return receiver instanceof hara.lang.protocol.IOFn;
      }
      if (!parents.isEmpty()) return true;
      if (name.endsWith(".IMutable") || name.endsWith("/IMutable")) {
        return receiver instanceof hara.lang.protocol.IMutable;
      }
      if (name.endsWith(".IPersistent") || name.endsWith("/IPersistent")) {
        return receiver instanceof hara.lang.protocol.IPersistent;
      }
      return false;
    }
    for (String methodName : methods.keySet()) {
      if (implementations.resolveExplicit(methodName, receiver) == null) return false;
    }
    return true;
  }

  @TruffleBoundary
  public Object invoke(String methodName, Object receiver, Object[] arguments) {
    HaraProtocolMethod method = method(methodName);
    if (method == null) {
      throw new HaraException("Unknown method " + name + "/" + methodName);
    }
    if (!method.acceptsCallArity(arguments.length + 1)) {
      throw new HaraException(
          "protocol/arity: "
              + name
              + "/"
              + methodName
              + " expects "
              + method.expectedCallArguments()
              + " arguments, received "
              + arguments.length);
    }
    HaraProtocolImplementation implementation = implementation(receiver, methodName);
    if (implementation == null) {
      throw new HaraException(
          "protocol/unsupported-receiver: No "
              + name
              + "/"
              + methodName
              + " implementation ("
              + HaraDispatchKey.describeReceiver(receiver)
              + ")");
    }
    try {
      return implementation.invoke(receiver, arguments);
    } catch (Ex.Unsupported error) {
      throw new HaraException(
          "protocol/unsupported-receiver: "
              + name
              + "/"
              + methodName
              + " does not support "
              + HaraDispatchKey.describeReceiver(receiver)
              + (error.getMessage() == null ? "" : " (" + error.getMessage() + ")"));
    }
  }

  private static HaraProtocolInvoker functionInvoker(HaraFunction function) {
    return new HaraProtocolInvoker() {
      @Override
      public Object invoke(Object receiver, Object[] arguments) {
        Object[] callArguments = new Object[arguments.length + 1];
        callArguments[0] = receiver;
        System.arraycopy(arguments, 0, callArguments, 1, arguments.length);
        return function.callTarget().call(function.callArguments(callArguments));
      }

      @Override
      public int arity() {
        return function.arity();
      }
    };
  }

  @ExportMessage
  @TruffleBoundary
  Object toDisplayString(boolean allowSideEffects) {
    return "#<protocol " + name + ">";
  }

  @Override
  public String toString() {
    return "#<protocol " + name + ">";
  }

  public static final class HaraProtocolMethod {
    private final HaraProtocol protocol;
    private final String name;
    private final int arity;

    private HaraProtocolMethod(HaraProtocol protocol, String name, int arity) {
      this.protocol = protocol;
      this.name = name;
      this.arity = arity;
    }

    public HaraProtocol protocol() {
      return protocol;
    }

    public String name() {
      return name;
    }

    public int arity() {
      return arity;
    }

    public boolean variadic() {
      return arity < 0;
    }

    public int minimumArity() {
      return arity < 0 ? 1 : arity;
    }

    public boolean acceptsCallArity(int candidate) {
      return candidate < 0 || (variadic() ? candidate >= minimumArity() : candidate == arity);
    }

    public String expectedCallArguments() {
      return variadic() ? "at least " + (minimumArity() - 1) : Integer.toString(arity - 1);
    }
  }
}
