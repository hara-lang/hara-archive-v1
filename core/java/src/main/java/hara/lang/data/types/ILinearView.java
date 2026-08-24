package hara.lang.data.types;

import hara.lang.protocol.ILinearType;

public interface ILinearView<E> extends ILinearType<E> {
  public ILinearType<E> subview(int start, int end);
}
