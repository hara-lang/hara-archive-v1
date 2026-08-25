package hara.lang.protocol;

import hara.lang.base.Iter;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;
import java.util.Iterator;

@HaraProtocolBinding(
    namespace = "std.protocol.icoll",
    name = "IColl",
    parents = {"IEquality", "IConj", "IEmpty", "IHash", "IDisplay"})
public interface IColl<E>
    extends Iterable<E>, IEquality, IConj<E>, IEmpty, IHash, IDisplay {

  @HaraMethod(value = "start-string", arity = 1)
  String startString();

  @HaraMethod(value = "end-string", arity = 1)
  String endString();

  @HaraMethod(value = "sep-string", arity = 1)
  default String sepString() {
    return " ";
  }

  @Override
  Iterator<E> iterator();

  @Override
  default String display() {
    return Iter.display(iterator(), startString(), endString(), sepString());
  }
}
