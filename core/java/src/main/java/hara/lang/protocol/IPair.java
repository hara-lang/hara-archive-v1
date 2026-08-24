package hara.lang.protocol;

import hara.lang.base.Ex;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

import java.util.Map;

@HaraProtocolBinding(namespace = "std.protocol.ipair", name = "IPair")
public interface IPair<K, V> extends Map.Entry<K, V> {
  @HaraMethod(value = "key", arity = 1)
  @Override
  K getKey();

  @HaraMethod(value = "value", arity = 1)
  @Override
  V getValue();

  @Override
  default V setValue(V value) {
    throw new Ex.Unsupported();
  }
}
