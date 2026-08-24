package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(
    namespace = "std.protocol.ihashcached", name = "IHashCached", parents = {"IHash"})
public interface IHashCached extends IHash {

  @HaraMethod(value = "hash-current", arity = 1)
  long hashCurrent();

  @Override
  default long hashGet() {
    long h = hashCurrent();
    if (h == 0) {
      h = hashCalc();
      hashPut(h);
    }
    return h;
  }

  @Override
  default long hashGet(Constant.HashType t) {
    return (hashType() == t) ? hashGet() : hashCalc(t);
  }

  @HaraMethod(value = "hash-put", arity = 2)
  void hashPut(long hash);
}
