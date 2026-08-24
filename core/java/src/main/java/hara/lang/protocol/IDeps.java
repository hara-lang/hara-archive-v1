package hara.lang.protocol;

import hara.lang.protocol.ISetType;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;
import java.util.Iterator;

/** Dependency lookup for books, snapshots, and similar stores. */
@HaraProtocolBinding(namespace = "std.protocol.ideps", name = "IDeps")
public interface IDeps<K, E> {
  @HaraMethod(value = "dep-get", arity = 2)
  E depGet(K key);

  @HaraMethod(value = "dep-entries", arity = 2)
  ISetType<K> depEntries(K key);

  @HaraMethod(value = "dep-keys", arity = 1)
  Iterator<K> depKeys();
}
