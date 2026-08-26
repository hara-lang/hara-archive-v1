package hara.truffle;

import com.oracle.truffle.api.interop.TruffleObject;
import hara.lang.data.HaraCharacter;
import java.util.Objects;

/** Stable registration keys for the language-agnostic protocol ABI. */
public final class HaraDispatchKey {
  public enum Kind {
    HARA_TYPE,
    JAVA_CLASS,
    PRIMITIVE,
    NIL,
    FOREIGN,
    DEFAULT
  }

  public enum PrimitiveCategory {
    BOOLEAN,
    NUMBER,
    CHARACTER,
    STRING
  }

  private final Kind kind;
  private final HaraType haraType;
  private final Class<?> javaClass;
  private final PrimitiveCategory primitiveCategory;

  private HaraDispatchKey(
      Kind kind, HaraType haraType, Class<?> javaClass, PrimitiveCategory primitiveCategory) {
    this.kind = kind;
    this.haraType = haraType;
    this.javaClass = javaClass;
    this.primitiveCategory = primitiveCategory;
  }

  public static HaraDispatchKey haraType(HaraType type) {
    return new HaraDispatchKey(Kind.HARA_TYPE, Objects.requireNonNull(type), null, null);
  }

  public static HaraDispatchKey javaClass(Class<?> type) {
    return new HaraDispatchKey(Kind.JAVA_CLASS, null, Objects.requireNonNull(type), null);
  }

  public static HaraDispatchKey primitive(PrimitiveCategory category) {
    return new HaraDispatchKey(Kind.PRIMITIVE, null, null, Objects.requireNonNull(category));
  }

  public static HaraDispatchKey nil() {
    return new HaraDispatchKey(Kind.NIL, null, null, null);
  }

  public static HaraDispatchKey foreign() {
    return new HaraDispatchKey(Kind.FOREIGN, null, null, null);
  }

  public static HaraDispatchKey defaultKey() {
    return new HaraDispatchKey(Kind.DEFAULT, null, null, null);
  }

  public Kind kind() {
    return kind;
  }

  HaraType haraType() {
    return haraType;
  }

  Class<?> javaClass() {
    return javaClass;
  }

  PrimitiveCategory primitiveCategory() {
    return primitiveCategory;
  }

  static PrimitiveCategory primitiveCategory(Object value) {
    if (value instanceof Boolean) {
      return PrimitiveCategory.BOOLEAN;
    }
    if (value instanceof Number) {
      return PrimitiveCategory.NUMBER;
    }
    if (value instanceof HaraCharacter || value instanceof Character) {
      return PrimitiveCategory.CHARACTER;
    }
    if (value instanceof CharSequence) {
      return PrimitiveCategory.STRING;
    }
    return null;
  }

  static boolean isForeign(Object value) {
    return value instanceof TruffleObject
        && !(value instanceof HaraStruct)
        && !(value instanceof HaraMutable)
        && !(value instanceof HaraFunction)
        && !(value instanceof HaraProtocol)
        && !(value instanceof HaraType)
        && !(value instanceof HaraVar)
        && !(value instanceof HaraBox);
  }

  public static String describeReceiver(Object value) {
    if (value == null) {
      return "category=nil, dispatch=nil -> default";
    }
    HaraType namedType = namedType(value);
    if (namedType != null) {
      HaraType type = namedType;
      return "category=hara-type, type=" + type.name()
          + ", class=" + value.getClass().getName()
          + ", dispatch=hara-type -> java-class -> default";
    }
    if (isForeign(value)) {
      return "category=foreign, class=" + value.getClass().getName()
          + ", dispatch=foreign -> java-class -> default";
    }
    PrimitiveCategory category = primitiveCategory(value);
    if (category != null) {
      return "category=" + category.name().toLowerCase()
          + ", class=" + value.getClass().getName()
          + ", dispatch=java-class -> primitive:" + category.name().toLowerCase()
          + " -> default";
    }
    return "category=java-class, class=" + value.getClass().getName()
        + ", dispatch=java-class -> default";
  }

  static HaraType namedType(Object value) {
    if (value instanceof HaraStruct struct) {
      return struct.type();
    }
    if (value instanceof HaraMutable mutable) {
      return mutable.type();
    }
    return null;
  }

  @Override
  public boolean equals(Object other) {
    if (!(other instanceof HaraDispatchKey)) {
      return false;
    }
    HaraDispatchKey that = (HaraDispatchKey) other;
    return kind == that.kind
        && haraType == that.haraType
        && javaClass == that.javaClass
        && primitiveCategory == that.primitiveCategory;
  }

  @Override
  public int hashCode() {
    return 31
            * (31 * (31 * kind.hashCode() + System.identityHashCode(haraType))
                + System.identityHashCode(javaClass))
        + Objects.hashCode(primitiveCategory);
  }

  @Override
  public String toString() {
    switch (kind) {
      case HARA_TYPE:
        return haraType.name();
      case JAVA_CLASS:
        return javaClass.getName();
      case PRIMITIVE:
        return primitiveCategory.name().toLowerCase();
      case NIL:
        return "nil";
      case FOREIGN:
        return "foreign";
      case DEFAULT:
        return "default";
      default:
        throw new AssertionError(kind);
    }
  }
}
