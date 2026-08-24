package hara.lang.protocol;

import java.util.List;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.ispace", name = "ISpace")
public interface ISpace {
  @HaraMethod(value = "context-set", arity = 4)
  void contextSet(Object context, Object key, Object options);

  @HaraMethod(value = "context-unset", arity = 2)
  void contextUnset(Object context);

  @HaraMethod(value = "context-list", arity = 1)
  List<?> contextList();

  @HaraMethod(value = "context-get", arity = 2)
  Object contextGet(Object context);

  @HaraMethod(value = "rt-active", arity = 1)
  List<?> activeRuntimes();

  @HaraMethod(value = "rt-get", arity = 2)
  Object runtimeGet(Object context);

  @HaraMethod(value = "rt-start", arity = 2)
  Object runtimeStart(Object context);

  @HaraMethod(value = "rt-started?", arity = 2)
  boolean runtimeStarted(Object context);

  @HaraMethod(value = "rt-stopped?", arity = 2)
  boolean runtimeStopped(Object context);

  @HaraMethod(value = "rt-stop", arity = 2)
  void runtimeStop(Object context);
}
