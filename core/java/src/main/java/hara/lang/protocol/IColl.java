package hara.lang.protocol;

import hara.lang.base.Iter;
import hara.lang.declaration.HaraHostSupport;

@HaraHostSupport(reason = "Java collection adapter helper; collection behavior is not a guest protocol")
public interface IColl<E>
    extends Iterable<E>, IEquality, IConj<E>, IEmpty, ICount, IHash, IDisplay {

  String startString();

  String endString();

  default String sepString() {
    return " ";
  }

  @Override
  default String display() {
    return Iter.display(iterator(), startString(), endString(), sepString());
  }
}
