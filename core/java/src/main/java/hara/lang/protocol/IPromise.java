package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** An asynchronous value with composable settlement handlers. */
@HaraProtocolBinding(
    namespace = "std.protocol.ipromise",
    name = "IPromise",
    parents = {"IDeref", "IDerefTimeout"})
public interface IPromise
    extends IDeref<Object>, IDerefTimeout<Object> {
  @HaraMethod(value = "state", arity = 1)
  Object state();

  @HaraMethod(value = "value", arity = 1)
  Object value();

  @HaraMethod(value = "then", arity = 2)
  Object then(Object function);

  @HaraMethod(value = "catch", arity = 2)
  Object catchError(Object function);

  @HaraMethod(value = "finally", arity = 2)
  Object finallyDo(Object function);

  @HaraMethod(value = "cancel", arity = 1)
  Object cancel();
}
