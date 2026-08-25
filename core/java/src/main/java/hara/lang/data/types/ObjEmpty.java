package hara.lang.data.types;

import hara.lang.base.Iter;
import hara.lang.protocol.IColl;
import hara.lang.protocol.ICount;
import hara.lang.protocol.IMetadata;
import hara.lang.protocol.INth;

import java.util.Iterator;

public abstract class ObjEmpty<E> extends ObjPersistent implements IColl<E>, ICount, INth<E> {

  public ObjEmpty() {
    super(null);
  }

  public ObjEmpty(IMetadata meta) {
    super(meta);
  }

  @Override
  public long count() {
    return 0;
  }

  @Override
  public ObjEmpty<E> empty() {
    return this;
  }

  @SuppressWarnings({"unchecked", "rawtypes"})
  @Override
  public Iterator iterator() {
    return Iter.emptyIterator();
  }

  @Override
  public E nth(long i) {
    return null;
  }
}
