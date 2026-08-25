package hara.truffle.bytecode;

import hara.truffle.HalcSchema;
import java.util.List;
import java.util.Objects;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Map;

/** Immutable, runtime-neutral representation of a validated HBC0 program. */
public record HbcProgram(
    String namespace,
    List<Object> constants,
    List<List<MetadataEntry>> varMetadata,
    Map<String, HalcSchema.Type> schemaTypes,
    Map<String, HalcSchema.Type> functionTypes,
    Map<String, HalcSchema.Type> inferredFunctionTypes,
    List<Function> functions,
    int entry) {

  public HbcProgram {
    constants = List.copyOf(constants);
    varMetadata = varMetadata.stream().map(List::copyOf).toList();
    schemaTypes = Map.copyOf(schemaTypes);
    functionTypes = Map.copyOf(functionTypes);
    inferredFunctionTypes = Map.copyOf(inferredFunctionTypes);
    functions = List.copyOf(functions);
  }

  /** Compatibility constructor for untyped HBC0-HBC0 programs and tests. */
  public HbcProgram(
      List<Object> constants,
      List<List<MetadataEntry>> varMetadata,
      List<Function> functions,
      int entry) {
    this(null, constants, varMetadata, Map.of(), Map.of(), Map.of(), functions, entry);
  }

  /** Resolves the normalized annotation attached to a compiled prototype. */
  public HalcSchema.Type functionSchema(int function) {
    if (function < 0 || function >= functions.size()) return null;
    String name = functions.get(function).name();
    if (name == null) return null;
    String qualified = name.contains("/") ? name : namespace == null ? null : namespace + "/" + name;
    if (qualified == null) return null;
    HalcSchema.Type schema = functionTypes.get(qualified);
    if (schema == null) schema = inferredFunctionTypes.get(qualified);
    java.util.HashSet<String> visited = new java.util.HashSet<>();
    while (schema instanceof HalcSchema.Reference reference) {
      if (!visited.add(reference.name())) return schema;
      HalcSchema.Type resolved = schemaTypes.get(reference.name());
      if (resolved == null) return schema;
      schema = resolved;
    }
    return schema;
  }

  public record Function(
      String name,
      boolean asyncFunction,
      int arity,
      boolean variadic,
      int captureCount,
      int localCount,
      int maxStack,
      List<Instruction> code,
      List<Position> sourceMap,
      List<TryEntry> handlers) {
    public Function {
      code = List.copyOf(code);
      sourceMap = Collections.unmodifiableList(new ArrayList<>(sourceMap));
      handlers = List.copyOf(handlers);
    }
  }

  /** A null entry represents an instruction without source information. */
  public record Position(long offset, long line, long column) {}

  public record TryEntry(
      long start,
      long end,
      int depth,
      List<CatchEntry> catches,
      Long finallyTarget,
      Integer pendingValue,
      Integer pendingError) {
    public TryEntry {
      catches = List.copyOf(catches);
    }
  }

  public record CatchEntry(String className, int binding, long target) {
    public CatchEntry {
      Objects.requireNonNull(className, "className");
    }
  }

  public record MetadataEntry(MetadataValue key, MetadataValue value) {}

  /** Tagged metadata value; payload shapes are fixed by {@link Kind}. */
  public record MetadataValue(Kind kind, Object value) {
    public MetadataValue {
      Objects.requireNonNull(kind, "kind");
      if (value instanceof List<?> values) value = List.copyOf(values);
    }

    public enum Kind {
      NIL,
      BOOLEAN,
      NUMBER,
      FLOAT,
      BIG_INTEGER,
      RESERVED_DECIMAL,
      CHARACTER,
      REGEX,
      TAGGED,
      STRING,
      KEYWORD,
      SYMBOL,
      VECTOR,
      LIST,
      SET,
      MAP
    }
  }

  public record TaggedMetadata(String tag, MetadataValue value) {}

  public record Instruction(Opcode opcode, long first, long second, long third) {
    public Instruction {
      Objects.requireNonNull(opcode, "opcode");
    }

    public static Instruction of(Opcode opcode) {
      return new Instruction(opcode, 0, 0, 0);
    }
  }

  public enum Primitive {
    ADD(0),
    SUBTRACT(1),
    MULTIPLY(2),
    DIVIDE(3),
    REMAINDER(4),
    EQUAL(5),
    LESS(6),
    LESS_OR_EQUAL(7),
    GREATER(8),
    GREATER_OR_EQUAL(9),
    COUNT(10),
    GET(11),
    META(12),
    NTH(13),
    ASSOC(14),
    FIRST(15),
    REST(16),
    SECOND(17),
    TO_MUTABLE(18),
    TO_PERSISTENT(19),
    NUMBER_PREDICATE(20),
    ARRAY_NEW(21),
    ARRAY_GET(22),
    ARRAY_SET(23),
    OBJECT_NEW(24),
    OBJECT_GET(25),
    OBJECT_SET(26),
    MODULO(27);

    private final int id;

    Primitive(int id) {
      this.id = id;
    }

    public int id() {
      return id;
    }

    public static Primitive fromId(int id) {
      for (Primitive primitive : values()) if (primitive.id == id) return primitive;
      throw new HbcFormatException("bytecode artifact contains an unknown primitive");
    }
  }

  /** Stable HBC0 opcode identifiers. Generated Truffle bytecodes are deliberately private. */
  public enum Opcode {
    CONSTANT(0),
    NIL(1),
    TRUE(2),
    FALSE(3),
    LOAD_LOCAL(4),
    STORE_LOCAL(5),
    POP(6),
    PRIMITIVE(7),
    JUMP(8),
    JUMP_IF_FALSE(9),
    CLOSURE(10),
    CALL(11),
    CALL_STATIC(12),
    THROW(13),
    RETHROW(14),
    GET_GLOBAL(15),
    DEF_GLOBAL(16),
    SET_GLOBAL(17),
    VAR_GLOBAL(18),
    DECLARE_GLOBAL(19),
    DEF_STRUCT(20),
    INSTANCE_OF(22),
    MAKE_MULTI_ARITY(23),
    RETURN(24),
    PRIMITIVE_LOCAL_CONST(25),
    AWAIT(26),
    HOST_CALL(27),
    DUP(28),
    BUILD_VECTOR(29),
    BUILD_MAP(30),
    BUILD_SET(31),
    DEF_MACRO(32),
    BUILD_LIST(33),
    CONCAT_LIST(34),
    TO_VECTOR(35),
    PRIMITIVE_VALUE(37),
    BUILTIN_VALUE(38),
    DYNAMIC_BIND(39),
    DYNAMIC_UNBIND(40),
    DEF_PROTOCOL(41),
    EXTEND_TYPE(42),
    DEF_MULTI(43),
    DEF_METHOD(44),
    DOT_CALL(45),
    YIELD(46),
    DEF_MUTABLE(47),
    MUTABLE_FIELD_GET(48),
    MUTABLE_FIELD_SET(49),
    INTRINSIC_CALL(50),
    INTRINSIC_VALUE(51),
    PROTOCOL_CALL(52);

    private final int id;

    Opcode(int id) {
      this.id = id;
    }

    public int id() {
      return id;
    }

    public static Opcode fromId(int id) {
      if (id == 21) {
        throw new HbcFormatException(
            "bytecode artifact uses retired StructField opcode 21; rebuild required");
      }
      for (Opcode opcode : values()) if (opcode.id == id) return opcode;
      throw new HbcFormatException("bytecode artifact contains an unknown opcode");
    }
  }
}
