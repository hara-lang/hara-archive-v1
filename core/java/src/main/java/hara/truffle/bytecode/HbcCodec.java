package hara.truffle.bytecode;

import hara.lang.data.Keyword;
import hara.lang.data.Symbol;
import hara.lang.base.NumUtils;
import hara.truffle.HalcSchema;
import hara.truffle.HtaValueCodec;
import hara.truffle.bytecode.HbcProgram.CatchEntry;
import hara.truffle.bytecode.HbcProgram.Function;
import hara.truffle.bytecode.HbcProgram.Instruction;
import hara.truffle.bytecode.HbcProgram.MetadataEntry;
import hara.truffle.bytecode.HbcProgram.MetadataValue;
import hara.truffle.bytecode.HbcProgram.Opcode;
import hara.truffle.bytecode.HbcProgram.Position;
import hara.truffle.bytecode.HbcProgram.Primitive;
import hara.truffle.bytecode.HbcProgram.TaggedMetadata;
import hara.truffle.bytecode.HbcProgram.TryEntry;
import java.io.ByteArrayOutputStream;
import java.math.BigInteger;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.CharacterCodingException;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.regex.Pattern;

/** Canonical HBC0 encoder/decoder shared with {@code rust/src/vm/artifact.rs}. */
public final class HbcCodec {
  private static final byte[] MAGIC = {'H', 'B', 'C', '0'};
  private static final int DIGEST_BYTES = 32;

  private HbcCodec() {}

  public static HbcProgram decode(byte[] artifact) {
    if (artifact.length < MAGIC.length + Integer.BYTES + DIGEST_BYTES) {
      throw malformed("bytecode artifact is truncated");
    }
    byte[] magic = Arrays.copyOf(artifact, MAGIC.length);
    if (!Arrays.equals(MAGIC, magic)) {
      throw malformed("bytecode artifact has invalid magic");
    }
    long payloadLength = Integer.toUnsignedLong(ByteBuffer.wrap(artifact, 4, 4).getInt());
    long payloadEnd = 8L + payloadLength;
    if (payloadEnd + DIGEST_BYTES != artifact.length) {
      throw malformed("bytecode artifact length mismatch");
    }
    byte[] payload = Arrays.copyOfRange(artifact, 8, (int) payloadEnd);
    byte[] expected = Arrays.copyOfRange(artifact, (int) payloadEnd, artifact.length);
    if (!MessageDigest.isEqual(sha256(payload), expected)) {
      throw malformed("bytecode artifact checksum mismatch");
    }

    Reader in = new Reader(payload);
    int entry = in.u16();
    String namespace = in.optionalString();
    List<Object> constants = in.many(reader -> HtaValueCodec.decodeCanonical(reader.bytes()));
    List<List<MetadataEntry>> metadata = in.many(HbcCodec::readMetadata);
    Map<String, HalcSchema.Type> schemaTypes = readSchemaMap(in);
    Map<String, HalcSchema.Type> functionTypes = readSchemaMap(in);
    Map<String, HalcSchema.Type> inferredFunctionTypes = readSchemaMap(in);
    List<Function> functions = in.many(HbcCodec::readFunction);
    in.finish();
    HbcProgram program =
        new HbcProgram(
            namespace,
            constants,
            metadata,
            schemaTypes,
            functionTypes,
            inferredFunctionTypes,
            functions,
            entry);
    HbcValidator.validate(program);
    return program;
  }

  public static byte[] encode(HbcProgram program) {
    HbcValidator.validate(program);
    Writer out = new Writer();
    out.u16(program.entry());
    out.optionalString(program.namespace());
    out.many(program.constants(), value -> out.bytes(HtaValueCodec.encode(value)));
    out.many(program.varMetadata(), entries -> writeMetadata(out, entries));
    writeSchemaMap(out, program.schemaTypes());
    writeSchemaMap(out, program.functionTypes());
    writeSchemaMap(out, program.inferredFunctionTypes());
    out.many(program.functions(), function -> writeFunction(out, function));
    byte[] payload = out.toByteArray();
    Writer artifact = new Writer();
    artifact.raw(MAGIC);
    artifact.u32(payload.length);
    artifact.raw(payload);
    artifact.raw(sha256(payload));
    return artifact.toByteArray();
  }

  private static Function readFunction(Reader in) {
    String name = in.optionalString();
    boolean asyncFunction = in.bool();
    int arity = in.u16();
    boolean variadic = in.bool();
    int captureCount = in.u16();
    int localCount = in.u16();
    int maxStack = in.u16();
    List<Instruction> code = in.many(HbcCodec::readInstruction);
    List<Position> sourceMap =
        in.many(reader -> reader.bool() ? new Position(reader.u32(), reader.u32(), reader.u32()) : null);
    List<TryEntry> handlers =
        in.many(
            reader -> {
              long start = reader.u32();
              long end = reader.u32();
              int depth = reader.u16();
              List<CatchEntry> catches =
                  reader.many(r -> new CatchEntry(r.string(), r.u16(), r.u32()));
              return new TryEntry(
                  start,
                  end,
                  depth,
                  catches,
                  reader.optionalU32(),
                  reader.optionalU16(),
                  reader.optionalU16());
            });
    return new Function(
        name,
        asyncFunction,
        arity,
        variadic,
        captureCount,
        localCount,
        maxStack,
        code,
        sourceMap,
        handlers);
  }

  private static void writeFunction(Writer out, Function function) {
    out.optionalString(function.name());
    out.bool(function.asyncFunction());
    out.u16(function.arity());
    out.bool(function.variadic());
    out.u16(function.captureCount());
    out.u16(function.localCount());
    out.u16(function.maxStack());
    out.many(function.code(), instruction -> writeInstruction(out, instruction));
    out.many(
        function.sourceMap(),
        position -> {
          out.bool(position != null);
          if (position != null) {
            out.u32(position.offset());
            out.u32(position.line());
            out.u32(position.column());
          }
        });
    out.many(
        function.handlers(),
        handler -> {
          out.u32(handler.start());
          out.u32(handler.end());
          out.u16(handler.depth());
          out.many(
              handler.catches(),
              clause -> {
                out.string(clause.className());
                out.u16(clause.binding());
                out.u32(clause.target());
              });
          out.optionalU32(handler.finallyTarget());
          out.optionalU16(handler.pendingValue());
          out.optionalU16(handler.pendingError());
        });
  }

  private static Instruction readInstruction(Reader in) {
    Opcode opcode = Opcode.fromId(in.u8());
    return switch (opcode) {
      case CONSTANT, JUMP, JUMP_IF_FALSE, GET_GLOBAL, SET_GLOBAL, VAR_GLOBAL,
          DECLARE_GLOBAL, MUTABLE_FIELD_GET, MUTABLE_FIELD_SET, BUILTIN_VALUE,
          DYNAMIC_BIND, DYNAMIC_UNBIND,
          DEF_PROTOCOL, EXTEND_TYPE, DEF_MULTI, DEF_METHOD ->
          new Instruction(opcode, in.u32(), 0, 0);
      case LOAD_LOCAL, STORE_LOCAL, BUILD_VECTOR, BUILD_MAP, BUILD_SET, BUILD_LIST,
          CONCAT_LIST -> new Instruction(opcode, in.u16(), 0, 0);
      case PRIMITIVE -> new Instruction(opcode, Primitive.fromId(in.u8()).id(), in.u8(), 0);
      case PRIMITIVE_LOCAL_CONST ->
          new Instruction(opcode, Primitive.fromId(in.u8()).id(), in.u16(), in.u32());
      case CLOSURE, CALL_STATIC -> new Instruction(opcode, in.u16(), in.u8(), 0);
      case CALL -> new Instruction(opcode, in.u8(), 0, 0);
      case DEF_GLOBAL, DEF_MACRO ->
          new Instruction(opcode, in.u32(), optionalSentinel(in.optionalU16()), 0);
      case DEF_STRUCT, DEF_MUTABLE -> new Instruction(opcode, in.u32(), in.u32(), 0);
      case MAKE_MULTI_ARITY -> new Instruction(opcode, in.u32(), in.u8(), 0);
      case PRIMITIVE_VALUE -> new Instruction(opcode, Primitive.fromId(in.u8()).id(), 0, 0);
      case DOT_CALL -> new Instruction(opcode, in.u32(), in.u8(), 0);
      case INTRINSIC_CALL, PROTOCOL_CALL -> new Instruction(opcode, in.u32(), in.u8(), 0);
      case INTRINSIC_VALUE -> new Instruction(opcode, in.u32(), 0, 0);
      default -> Instruction.of(opcode);
    };
  }

  private static void writeInstruction(Writer out, Instruction instruction) {
    Opcode opcode = instruction.opcode();
    out.u8(opcode.id());
    switch (opcode) {
      case CONSTANT, JUMP, JUMP_IF_FALSE, GET_GLOBAL, SET_GLOBAL, VAR_GLOBAL,
          DECLARE_GLOBAL, MUTABLE_FIELD_GET, MUTABLE_FIELD_SET, BUILTIN_VALUE,
          DYNAMIC_BIND, DYNAMIC_UNBIND,
          DEF_PROTOCOL, EXTEND_TYPE, DEF_MULTI, DEF_METHOD -> out.u32(instruction.first());
      case LOAD_LOCAL, STORE_LOCAL, BUILD_VECTOR, BUILD_MAP, BUILD_SET, BUILD_LIST,
          CONCAT_LIST -> out.u16(instruction.first());
      case PRIMITIVE -> {
        out.u8(instruction.first());
        out.u8(instruction.second());
      }
      case PRIMITIVE_LOCAL_CONST -> {
        out.u8(instruction.first());
        out.u16(instruction.second());
        out.u32(instruction.third());
      }
      case CLOSURE, CALL_STATIC -> {
        out.u16(instruction.first());
        out.u8(instruction.second());
      }
      case CALL -> out.u8(instruction.first());
      case DEF_GLOBAL, DEF_MACRO -> {
        out.u32(instruction.first());
        out.optionalU16(fromOptionalSentinel(instruction.second()));
      }
      case DEF_STRUCT, DEF_MUTABLE -> {
        out.u32(instruction.first());
        out.u32(instruction.second());
      }
      case MAKE_MULTI_ARITY -> {
        out.u32(instruction.first());
        out.u8(instruction.second());
      }
      case PRIMITIVE_VALUE -> out.u8(instruction.first());
      case DOT_CALL -> {
        out.u32(instruction.first());
        out.u8(instruction.second());
      }
      case INTRINSIC_CALL, PROTOCOL_CALL -> {
        out.u32(instruction.first());
        out.u8(instruction.second());
      }
      case INTRINSIC_VALUE -> out.u32(instruction.first());
      default -> {}
    }
  }

  private static void writeSchemaMap(Writer out, Map<String, HalcSchema.Type> schemas) {
    List<String> names = schemas.keySet().stream().sorted().toList();
    out.u32(names.size());
    for (String name : names) {
      out.string(name);
      writeSchemaType(out, schemas.get(name));
    }
  }

  private static Map<String, HalcSchema.Type> readSchemaMap(Reader in) {
    Map<String, HalcSchema.Type> schemas = new LinkedHashMap<>();
    for (Map.Entry<String, HalcSchema.Type> entry :
        in.many(reader -> Map.entry(reader.string(), readSchemaType(reader)))) {
      if (schemas.put(entry.getKey(), entry.getValue()) != null) {
        throw malformed("bytecode artifact contains duplicate schema " + entry.getKey());
      }
    }
    return Map.copyOf(schemas);
  }

  private static void writeSchemaType(Writer out, HalcSchema.Type schema) {
    switch (schema) {
      case HalcSchema.Primitive primitive -> {
        out.u8(0);
        out.string(primitive.name());
      }
      case HalcSchema.Reference reference -> {
        out.u8(1);
        out.string(reference.name());
      }
      case HalcSchema.Union union -> {
        out.u8(2);
        out.many(union.types(), type -> writeSchemaType(out, type));
      }
      case HalcSchema.VectorType vector -> {
        out.u8(3);
        writeSchemaType(out, vector.item());
      }
      case HalcSchema.SetType set -> {
        out.u8(10);
        writeSchemaType(out, set.item());
      }
      case HalcSchema.Tuple tuple -> {
        out.u8(4);
        out.many(tuple.items(), type -> writeSchemaType(out, type));
      }
      case HalcSchema.MapType map -> {
        // Preserve tag 5 for property-free maps so existing HBC artifacts remain stable.
        boolean propertyAware = map.fields().stream().anyMatch(field -> field.properties() != null);
        out.u8(propertyAware ? 12 : 5);
        out.many(
            map.fields(),
            field -> {
              writeSchemaSurface(out, field.name());
              if (propertyAware) {
                out.bool(field.properties() != null);
                if (field.properties() != null) writeSchemaSurface(out, field.properties());
              }
              writeSchemaType(out, field.type());
            });
      }
      case HalcSchema.StructType struct -> {
        out.u8(13);
        out.string(struct.name());
        out.bool(struct.mutable());
        boolean propertyAware = struct.fields().stream().anyMatch(field -> field.properties() != null);
        out.bool(propertyAware);
        out.many(
            struct.fields(),
            field -> {
              writeSchemaSurface(out, field.name());
              if (propertyAware) {
                out.bool(field.properties() != null);
                if (field.properties() != null) writeSchemaSurface(out, field.properties());
              }
              writeSchemaType(out, field.type());
            });
      }
      case HalcSchema.Properties properties -> {
        out.u8(11);
        writeSchemaType(out, properties.schema());
        writeSchemaSurface(out, properties.properties());
      }
      case HalcSchema.FunctionType function -> {
        out.u8(6);
        out.many(
            function.arities(),
            arity -> {
              out.many(arity.fixed(), type -> writeSchemaType(out, type));
              out.bool(arity.rest() != null);
              if (arity.rest() != null) writeSchemaType(out, arity.rest());
              writeSchemaType(out, arity.output());
            });
      }
      case HalcSchema.EnumType enumeration -> {
        out.u8(7);
        writeSchemaSurfaces(out, enumeration.values());
      }
      case HalcSchema.Extension extension -> {
        out.u8(8);
        out.string(extension.head());
        writeSchemaSurfaces(out, extension.arguments());
      }
      case HalcSchema.Unknown unknown -> {
        out.u8(9);
        writeSchemaSurface(out, unknown.surface());
      }
    }
  }

  private static HalcSchema.Type readSchemaType(Reader in) {
    return switch (in.u8()) {
      case 0 -> new HalcSchema.Primitive(in.string());
      case 1 -> new HalcSchema.Reference(in.string());
      case 2 -> new HalcSchema.Union(in.many(HbcCodec::readSchemaType));
      case 3 -> new HalcSchema.VectorType(readSchemaType(in));
      case 4 -> new HalcSchema.Tuple(in.many(HbcCodec::readSchemaType));
      case 5 ->
          new HalcSchema.MapType(
              in.many(
                  reader ->
                      new HalcSchema.Field(
                          readSchemaSurface(reader), null, readSchemaType(reader))));
      case 6 ->
          new HalcSchema.FunctionType(
              in.many(
                  reader -> {
                    List<HalcSchema.Type> fixed = reader.many(HbcCodec::readSchemaType);
                    HalcSchema.Type rest = reader.bool() ? readSchemaType(reader) : null;
                    return new HalcSchema.Function(fixed, rest, readSchemaType(reader));
                  }));
      case 7 -> new HalcSchema.EnumType(readSchemaSurfaces(in));
      case 8 -> new HalcSchema.Extension(in.string(), readSchemaSurfaces(in));
      case 9 -> new HalcSchema.Unknown(readSchemaSurface(in));
      case 10 -> new HalcSchema.SetType(readSchemaType(in));
      case 11 -> new HalcSchema.Properties(readSchemaType(in), readSchemaSurface(in));
      case 12 ->
          new HalcSchema.MapType(
              in.many(
                  reader -> {
                    Object name = readSchemaSurface(reader);
                    Object properties = reader.bool() ? readSchemaSurface(reader) : null;
                    return new HalcSchema.Field(name, properties, readSchemaType(reader));
                  }));
      case 13 -> {
        String name = in.string();
        boolean mutable = in.bool();
        boolean propertyAware = in.bool();
        yield new HalcSchema.StructType(
            name,
            mutable,
            in.many(
                reader -> {
                  Object fieldName = readSchemaSurface(reader);
                  Object properties =
                      propertyAware && reader.bool() ? readSchemaSurface(reader) : null;
                  return new HalcSchema.Field(fieldName, properties, readSchemaType(reader));
                }));
      }
      default -> throw malformed("bytecode artifact contains unknown schema type");
    };
  }

  private static void writeSchemaSurfaces(Writer out, List<?> values) {
    out.u32(values.size());
    for (Object value : values) writeSchemaSurface(out, value);
  }

  private static List<Object> readSchemaSurfaces(Reader in) {
    return in.many(HbcCodec::readSchemaSurface);
  }

  private static void writeSchemaSurface(Writer out, Object value) {
    out.string(HalcSchema.displaySurface(value));
  }

  private static Object readSchemaSurface(Reader in) {
    try {
      return HalcSchema.readSurface(in.string());
    } catch (RuntimeException error) {
      throw malformed("bytecode artifact contains invalid schema form: " + error.getMessage());
    }
  }

  private static List<MetadataEntry> readMetadata(Reader in) {
    return in.many(reader -> new MetadataEntry(readMetadataValue(reader), readMetadataValue(reader)));
  }

  private static void writeMetadata(Writer out, List<MetadataEntry> entries) {
    out.many(
        entries,
        entry -> {
          writeMetadataValue(out, entry.key());
          writeMetadataValue(out, entry.value());
        });
  }

  private static MetadataValue readMetadataValue(Reader in) {
    MetadataValue.Kind kind = metadataKind(in.u8());
    if (kind == MetadataValue.Kind.BIG_INTEGER) {
      return readMetadataBigInteger(in.string());
    }
    Object value =
        switch (kind) {
          case NIL -> null;
          case BOOLEAN -> in.bool();
          case NUMBER -> in.i64();
          case FLOAT -> {
            double floating = Double.longBitsToDouble(in.u64());
            if (!Double.isFinite(floating)) throw malformed("non-finite number");
            yield floating;
          }
          case BIG_INTEGER -> throw new AssertionError("big integer metadata was handled above");
          case RESERVED_DECIMAL -> throw malformed("bytecode artifact contains reserved decimal metadata");
          case CHARACTER -> requireUnicodeScalar(Math.toIntExact(in.u32()));
          case REGEX -> Pattern.compile(in.string());
          case TAGGED -> new TaggedMetadata(in.string(), readMetadataValue(in));
          case STRING -> in.string();
          case KEYWORD -> Keyword.create(in.string());
          case SYMBOL -> Symbol.create(in.string());
          case VECTOR, LIST, SET -> in.many(HbcCodec::readMetadataValue);
          case MAP -> in.many(reader -> new MetadataEntry(readMetadataValue(reader), readMetadataValue(reader)));
        };
    return new MetadataValue(kind, value);
  }

  private static MetadataValue readMetadataBigInteger(String text) {
    try {
      BigInteger value = new BigInteger(text);
      if (NumUtils.isLongValue(value)) {
        return new MetadataValue(MetadataValue.Kind.NUMBER, value.longValue());
      }
      return new MetadataValue(MetadataValue.Kind.BIG_INTEGER, value);
    } catch (NumberFormatException error) {
      throw malformed("bytecode artifact contains invalid big integer metadata");
    }
  }

  @SuppressWarnings("unchecked")
  private static void writeMetadataValue(Writer out, MetadataValue metadata) {
    if (metadata.kind() == MetadataValue.Kind.BIG_INTEGER
        && metadata.value() instanceof BigInteger integer
        && NumUtils.isLongValue(integer)) {
      out.u8(MetadataValue.Kind.NUMBER.ordinal());
      out.i64(integer.longValue());
      return;
    }
    out.u8(metadata.kind().ordinal());
    Object value = metadata.value();
    switch (metadata.kind()) {
      case NIL -> {}
      case BOOLEAN -> out.bool((Boolean) value);
      case NUMBER -> out.i64(((Number) value).longValue());
      case FLOAT -> {
        double floating = ((Number) value).doubleValue();
        if (!Double.isFinite(floating)) throw malformed("non-finite number");
        out.u64(Double.doubleToRawLongBits(floating));
      }
      case BIG_INTEGER -> out.string(value.toString());
      case RESERVED_DECIMAL -> throw new AssertionError("reserved decimal metadata cannot be encoded");
      case CHARACTER -> out.u32(((Number) value).longValue());
      case REGEX -> out.string(((Pattern) value).pattern());
      case TAGGED -> {
        TaggedMetadata tagged = (TaggedMetadata) value;
        out.string(tagged.tag());
        writeMetadataValue(out, tagged.value());
      }
      case STRING -> out.string((String) value);
      case KEYWORD -> {
        Keyword keyword = (Keyword) value;
        out.string(qualified(keyword.getNamespace(), keyword.getName()));
      }
      case SYMBOL -> {
        Symbol symbol = (Symbol) value;
        out.string(qualified(symbol.getNamespace(), symbol.getName()));
      }
      case VECTOR, LIST, SET ->
          out.many((List<MetadataValue>) value, item -> writeMetadataValue(out, item));
      case MAP -> out.many((List<MetadataEntry>) value, entry -> {
        writeMetadataValue(out, entry.key());
        writeMetadataValue(out, entry.value());
      });
    }
  }

  private static MetadataValue.Kind metadataKind(int tag) {
    MetadataValue.Kind[] kinds = MetadataValue.Kind.values();
    if (tag >= kinds.length) throw malformed("bytecode artifact contains unknown metadata");
    return kinds[tag];
  }

  private static String qualified(String namespace, String name) {
    return namespace == null ? name : namespace + "/" + name;
  }

  private static int requireUnicodeScalar(int value) {
    if (!Character.isValidCodePoint(value)
        || (value >= Character.MIN_SURROGATE && value <= Character.MAX_SURROGATE)) {
      throw malformed("bytecode artifact contains invalid character scalar");
    }
    return value;
  }

  private static long optionalSentinel(Integer value) {
    return value == null ? -1 : value;
  }

  private static Integer fromOptionalSentinel(long value) {
    return value < 0 ? null : Math.toIntExact(value);
  }

  private static byte[] sha256(byte[] bytes) {
    try {
      return MessageDigest.getInstance("SHA-256").digest(bytes);
    } catch (NoSuchAlgorithmException impossible) {
      throw new AssertionError(impossible);
    }
  }

  private static HbcFormatException malformed(String message) {
    return new HbcFormatException(message);
  }

  @FunctionalInterface
  private interface ReaderFunction<T> {
    T read(Reader reader);
  }

  @FunctionalInterface
  private interface WriterConsumer<T> {
    void write(T value);
  }

  private static final class Reader {
    private final ByteBuffer input;

    Reader(byte[] bytes) {
      input = ByteBuffer.wrap(bytes).order(ByteOrder.BIG_ENDIAN);
    }

    int u8() {
      require(1);
      return Byte.toUnsignedInt(input.get());
    }

    boolean bool() {
      return switch (u8()) {
        case 0 -> false;
        case 1 -> true;
        default -> throw malformed("bytecode artifact contains invalid boolean");
      };
    }

    int u16() {
      require(2);
      return Short.toUnsignedInt(input.getShort());
    }

    long u32() {
      require(4);
      return Integer.toUnsignedLong(input.getInt());
    }

    long u64() {
      require(8);
      return input.getLong();
    }

    long i64() {
      require(8);
      return input.getLong();
    }

    byte[] bytes() {
      int size = checkedSize(u32());
      require(size);
      byte[] result = new byte[size];
      input.get(result);
      return result;
    }

    String string() {
      try {
        return StandardCharsets.UTF_8
            .newDecoder()
            .onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT)
            .decode(ByteBuffer.wrap(bytes()))
            .toString();
      } catch (CharacterCodingException error) {
        throw malformed("bytecode artifact contains invalid UTF-8");
      }
    }

    String optionalString() {
      return bool() ? string() : null;
    }

    Integer optionalU16() {
      return bool() ? u16() : null;
    }

    Long optionalU32() {
      return bool() ? u32() : null;
    }

    <T> List<T> many(ReaderFunction<T> function) {
      int size = checkedSize(u32());
      ArrayList<T> values = new ArrayList<>(Math.min(size, 4096));
      for (int i = 0; i < size; i++) values.add(function.read(this));
      return values;
    }

    void finish() {
      if (input.hasRemaining()) throw malformed("bytecode artifact has trailing payload bytes");
    }

    private int checkedSize(long size) {
      if (size > Integer.MAX_VALUE) throw malformed("bytecode artifact length overflow");
      return (int) size;
    }

    private void require(int size) {
      if (size < 0 || input.remaining() < size) throw malformed("bytecode artifact is truncated");
    }
  }

  private static final class Writer {
    private final ByteArrayOutputStream output = new ByteArrayOutputStream();

    void u8(long value) {
      requireUnsigned(value, 0xffL, "u8");
      output.write((int) value);
    }

    void bool(boolean value) {
      u8(value ? 1 : 0);
    }

    void u16(long value) {
      requireUnsigned(value, 0xffffL, "u16");
      output.write((int) (value >>> 8));
      output.write((int) value);
    }

    void u32(long value) {
      requireUnsigned(value, 0xffff_ffffL, "u32");
      for (int shift = 24; shift >= 0; shift -= 8) output.write((int) (value >>> shift));
    }

    void u64(long value) {
      for (int shift = 56; shift >= 0; shift -= 8) output.write((int) (value >>> shift));
    }

    void i64(long value) {
      u64(value);
    }

    void bytes(byte[] value) {
      u32(value.length);
      raw(value);
    }

    void string(String value) {
      bytes(value.getBytes(StandardCharsets.UTF_8));
    }

    void optionalString(String value) {
      bool(value != null);
      if (value != null) string(value);
    }

    void optionalU16(Integer value) {
      bool(value != null);
      if (value != null) u16(value);
    }

    void optionalU32(Long value) {
      bool(value != null);
      if (value != null) u32(value);
    }

    <T> void many(List<T> values, WriterConsumer<T> consumer) {
      u32(values.size());
      for (T value : values) consumer.write(value);
    }

    void raw(byte[] value) {
      output.writeBytes(value);
    }

    byte[] toByteArray() {
      return output.toByteArray();
    }

    private void requireUnsigned(long value, long maximum, String type) {
      if (value < 0 || value > maximum) throw malformed("bytecode field does not fit " + type);
    }
  }
}
