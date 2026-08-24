package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.icontextlifecycle", name = "IContextLifeCycle")
public interface IContextLifeCycle {
  @HaraMethod(value = "has-module?", arity = 2)
  boolean hasModule(Object moduleId);

  @HaraMethod(value = "setup-module", arity = 2)
  void setupModule(Object moduleId);

  @HaraMethod(value = "teardown-module", arity = 2)
  void teardownModule(Object moduleId);

  @HaraMethod(value = "has-pointer?", arity = 2)
  boolean hasPointer(IPointer pointer);

  @HaraMethod(value = "setup-pointer", arity = 2)
  void setupPointer(IPointer pointer);

  @HaraMethod(value = "teardown-pointer", arity = 2)
  void teardownPointer(IPointer pointer);
}
