package hara.truffle;

import com.oracle.truffle.api.CompilerDirectives.TruffleBoundary;
import com.oracle.truffle.api.interop.InteropLibrary;
import com.oracle.truffle.api.interop.TruffleObject;
import com.oracle.truffle.api.interop.UnsupportedMessageException;
import com.oracle.truffle.api.library.ExportLibrary;
import com.oracle.truffle.api.library.ExportMessage;
import java.math.BigDecimal;
import java.math.BigInteger;

/** Exact polyglot number representation for integers outside the primitive long range. */
@ExportLibrary(InteropLibrary.class)
public final class HaraBigInteger implements TruffleObject {
  private static final BigInteger BYTE_MIN = BigInteger.valueOf(Byte.MIN_VALUE);
  private static final BigInteger BYTE_MAX = BigInteger.valueOf(Byte.MAX_VALUE);
  private static final BigInteger SHORT_MIN = BigInteger.valueOf(Short.MIN_VALUE);
  private static final BigInteger SHORT_MAX = BigInteger.valueOf(Short.MAX_VALUE);
  private static final BigInteger INT_MIN = BigInteger.valueOf(Integer.MIN_VALUE);
  private static final BigInteger INT_MAX = BigInteger.valueOf(Integer.MAX_VALUE);
  private static final BigInteger LONG_MIN = BigInteger.valueOf(Long.MIN_VALUE);
  private static final BigInteger LONG_MAX = BigInteger.valueOf(Long.MAX_VALUE);

  private final BigInteger value;

  public HaraBigInteger(BigInteger value) {
    if (value == null) throw new NullPointerException("value");
    if (value.compareTo(LONG_MIN) >= 0 && value.compareTo(LONG_MAX) <= 0) {
      throw new IllegalArgumentException("HaraBigInteger requires a value outside signed 64-bit range");
    }
    this.value = value;
  }

  public BigInteger value() {
    return value;
  }

  @ExportMessage
  boolean isNumber() {
    return true;
  }

  @ExportMessage
  boolean fitsInByte() {
    return between(BYTE_MIN, BYTE_MAX);
  }

  @ExportMessage
  boolean fitsInShort() {
    return between(SHORT_MIN, SHORT_MAX);
  }

  @ExportMessage
  boolean fitsInInt() {
    return between(INT_MIN, INT_MAX);
  }

  @ExportMessage
  boolean fitsInLong() {
    return between(LONG_MIN, LONG_MAX);
  }

  @ExportMessage
  boolean fitsInBigInteger() {
    return true;
  }

  @ExportMessage
  @TruffleBoundary
  boolean fitsInFloat() {
    float converted = value.floatValue();
    return Float.isFinite(converted)
        && new BigDecimal(Float.toString(converted)).toBigInteger().equals(value);
  }

  @ExportMessage
  @TruffleBoundary
  boolean fitsInDouble() {
    double converted = value.doubleValue();
    return Double.isFinite(converted) && BigDecimal.valueOf(converted).toBigInteger().equals(value);
  }

  @ExportMessage
  @TruffleBoundary
  byte asByte() throws UnsupportedMessageException {
    if (!fitsInByte()) throw UnsupportedMessageException.create();
    return value.byteValue();
  }

  @ExportMessage
  @TruffleBoundary
  short asShort() throws UnsupportedMessageException {
    if (!fitsInShort()) throw UnsupportedMessageException.create();
    return value.shortValue();
  }

  @ExportMessage
  @TruffleBoundary
  int asInt() throws UnsupportedMessageException {
    if (!fitsInInt()) throw UnsupportedMessageException.create();
    return value.intValue();
  }

  @ExportMessage
  @TruffleBoundary
  long asLong() throws UnsupportedMessageException {
    if (!fitsInLong()) throw UnsupportedMessageException.create();
    return value.longValue();
  }

  @ExportMessage
  BigInteger asBigInteger() {
    return value;
  }

  @ExportMessage
  @TruffleBoundary
  float asFloat() throws UnsupportedMessageException {
    if (!fitsInFloat()) throw UnsupportedMessageException.create();
    return value.floatValue();
  }

  @ExportMessage
  @TruffleBoundary
  double asDouble() throws UnsupportedMessageException {
    if (!fitsInDouble()) throw UnsupportedMessageException.create();
    return value.doubleValue();
  }

  @ExportMessage
  @TruffleBoundary
  Object toDisplayString(boolean allowSideEffects) {
    return value.toString();
  }

  @TruffleBoundary
  @Override
  public boolean equals(Object other) {
    return other instanceof HaraBigInteger && value.equals(((HaraBigInteger) other).value);
  }

  @TruffleBoundary
  @Override
  public int hashCode() {
    return value.hashCode();
  }

  @TruffleBoundary
  @Override
  public String toString() {
    return value.toString();
  }

  @TruffleBoundary
  private boolean between(BigInteger minimum, BigInteger maximum) {
    return value.compareTo(minimum) >= 0 && value.compareTo(maximum) <= 0;
  }
}
