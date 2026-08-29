package hara.lang.protocol;

public interface Constant {

  public static final Object[] EMPTY_ARRAY = new Object[] {};
  public static final Boolean F = Boolean.FALSE;
  public static final Boolean T = Boolean.TRUE;

  public enum MetaType {
    OBJECT,
    MAP,
    STRING
  }

  public enum HashType {
    SYSTEM,
    RAPID,
    MURMUR3,
    SIP
  };

  public enum ObjType {
    CLASS,
    TYPE,
    NIL,
    BOOLEAN,
    NUMBER,
    CHARACTER,
    STRING,
    SYMBOL,
    KEYWORD,
    PATTERN,
    DATE,
    UUID,
    URI,
    SEQUENTIAL,
    LIST,
    VECTOR,
    MAP_ENTRY,
    MAP,
    SET,
    FUNCTION,
    ATOM,
    META,
    OBJECT,
    ITERATOR,
    FUTURE,
    PROMISE,
    DELAY,
    PENDING,
    ERROR,
    READER,
    POINTER
  }
}
