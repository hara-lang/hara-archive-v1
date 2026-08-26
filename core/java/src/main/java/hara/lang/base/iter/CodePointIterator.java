package hara.lang.base.iter;

import hara.lang.data.HaraCharacter;
import java.util.Iterator;
import java.util.NoSuchElementException;

/** Iterator over Unicode scalar values rather than UTF-16 code units. */
public final class CodePointIterator implements Iterator<HaraCharacter> {
  private final String value;
  private int offset;

  public CodePointIterator(String value) {
    this.value = value;
  }

  @Override
  public boolean hasNext() {
    return offset < value.length();
  }

  @Override
  public HaraCharacter next() {
    if (!hasNext()) throw new NoSuchElementException();
    int codePoint = value.codePointAt(offset);
    offset += Character.charCount(codePoint);
    return HaraCharacter.of(codePoint);
  }

  @Override
  public void remove() {
    throw new UnsupportedOperationException("remove() not supported");
  }
}
