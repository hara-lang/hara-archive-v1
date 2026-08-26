package hara.lang.data;

import hara.lang.base.G;
import hara.lang.protocol.IDisplay;

/** Immutable Hara character scalar, represented by one Unicode code point. */
public final class HaraCharacter implements IDisplay, Comparable<HaraCharacter> {
  private final int codePoint;

  private HaraCharacter(int codePoint) {
    this.codePoint = codePoint;
  }

  public static HaraCharacter of(int codePoint) {
    if (!Character.isValidCodePoint(codePoint)
        || (codePoint >= Character.MIN_SURROGATE && codePoint <= Character.MAX_SURROGATE)) {
      throw new IllegalArgumentException("invalid character scalar: " + codePoint);
    }
    return new HaraCharacter(codePoint);
  }

  public int codePoint() {
    return codePoint;
  }

  public String text() {
    return new String(Character.toChars(codePoint));
  }

  @Override
  public String display() {
    return G.displayCharacter(codePoint);
  }

  @Override
  public int compareTo(HaraCharacter other) {
    return Integer.compare(codePoint, other.codePoint);
  }

  @Override
  public boolean equals(Object other) {
    return other instanceof HaraCharacter character && codePoint == character.codePoint;
  }

  @Override
  public int hashCode() {
    return Integer.hashCode(codePoint);
  }

  @Override
  public String toString() {
    return text();
  }
}
