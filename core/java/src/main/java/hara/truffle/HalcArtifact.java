package hara.truffle;

import hara.lang.data.Keyword;
import hara.lang.data.Symbol;
import hara.lang.data.HaraCharacter;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.ISetType;
import hara.lang.protocol.IMetadata;
import hara.lang.protocol.IObjType;
import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.EOFException;
import java.io.IOException;
import java.math.BigInteger;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.ArrayDeque;
import java.util.Deque;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Map;
import java.util.Map.Entry;
import java.util.Set;

/** Deterministic, host-neutral binary representation of a Hara source module. */
final class HalcArtifact {
  static final int FORMAT_VERSION = 1;
  static final int EXECUTABLE_FOUNDATION_FLAG = 1;
  static final String FOUNDATION_RESOURCE = "std/foundation.hal";
  static final String FOUNDATION_HALC_RESOURCE = "std/foundation.halc";

  private static final byte[] MAGIC = {'H', 'A', 'L', 'C'};
  private static final byte[] LEGACY_MAGIC = {'H', 'I', 'R', 0};
  private static final int HASH_BYTES = 32;
  private static final int MAX_PAYLOAD_BYTES = 64 * 1024 * 1024;
  private static final int MAX_COLLECTION_ITEMS = 1_000_000;

  private static final int NIL = 0;
  private static final int FALSE = 1;
  private static final int TRUE = 2;
  private static final int LONG = 3;
  private static final int DOUBLE = 4;
  private static final int BIG_INTEGER = 5;
  private static final int STRING = 6;
  private static final int CHARACTER = 8;
  private static final int SYMBOL = 9;
  private static final int KEYWORD = 10;
  private static final int LIST = 11;
  private static final int VECTOR = 12;
  private static final int MAP = 13;
  private static final int SET = 14;
  private static final int ORDERED_MAP = 15;
  private static final int ORDERED_SET = 16;
  private static final int REGEX = 17;

  private HalcArtifact() {}

  static byte[] encode(String namespace, String resource, byte[] source, Object[] forms) {
    try {
      forms = canonicalizeSchemaReferences(namespace, forms);
      buildSchemaIndex(namespace, forms);
      ByteArrayOutputStream payloadBytes = new ByteArrayOutputStream();
      try (DataOutputStream payload = new DataOutputStream(payloadBytes)) {
        writeString(payload, namespace);
        writeString(payload, resource);
        payload.write(sha256(source));
        writeCount(payload, forms.length);
        for (Object form : forms) writeValue(payload, form);
      }
      byte[] encodedPayload = payloadBytes.toByteArray();
      ByteArrayOutputStream artifactBytes = new ByteArrayOutputStream();
      try (DataOutputStream artifact = new DataOutputStream(artifactBytes)) {
        artifact.write(MAGIC);
        artifact.writeShort(FORMAT_VERSION);
        artifact.writeShort(EXECUTABLE_FOUNDATION_FLAG);
        artifact.writeInt(encodedPayload.length);
        artifact.write(sha256(encodedPayload));
        artifact.write(encodedPayload);
      }
      return artifactBytes.toByteArray();
    } catch (IOException error) {
      throw new HaraException("Unable to encode HALC: " + error.getMessage());
    }
  }

  static Module decode(byte[] artifactBytes) {
    try (DataInputStream artifact =
        new DataInputStream(new ByteArrayInputStream(artifactBytes))) {
      byte[] magic = artifact.readNBytes(MAGIC.length);
      Origin origin;
      if (Arrays.equals(MAGIC, magic)) {
        origin = Origin.HALC;
      } else if (Arrays.equals(LEGACY_MAGIC, magic)) {
        origin = Origin.LEGACY_HIR;
      } else {
        throw invalid("bad magic");
      }
      int version = artifact.readUnsignedShort();
      if (version != FORMAT_VERSION) {
        throw invalid("unsupported format version " + version);
      }
      int flags = artifact.readUnsignedShort();
      if (flags != EXECUTABLE_FOUNDATION_FLAG) {
        throw invalid("unsupported flags " + flags);
      }
      int payloadLength = artifact.readInt();
      if (payloadLength < 0 || payloadLength > MAX_PAYLOAD_BYTES) {
        throw invalid("invalid payload length " + payloadLength);
      }
      byte[] expectedHash = artifact.readNBytes(HASH_BYTES);
      if (expectedHash.length != HASH_BYTES) throw invalid("truncated payload hash");
      byte[] payloadBytes = artifact.readNBytes(payloadLength);
      if (payloadBytes.length != payloadLength) throw invalid("truncated payload");
      if (artifact.read() != -1) throw invalid("trailing bytes");
      if (!MessageDigest.isEqual(expectedHash, sha256(payloadBytes))) {
        throw invalid("payload checksum mismatch");
      }
      try (DataInputStream payload =
          new DataInputStream(new ByteArrayInputStream(payloadBytes))) {
        String namespace = readString(payload);
        String resource = readString(payload);
        byte[] sourceHash = payload.readNBytes(HASH_BYTES);
        if (sourceHash.length != HASH_BYTES) throw invalid("truncated source hash");
        int count = readCount(payload);
        Object[] forms = new Object[count];
        for (int index = 0; index < count; index++) forms[index] = readValue(payload);
        if (payload.read() != -1) throw invalid("trailing payload bytes");
        forms = canonicalizeSchemaReferences(namespace, forms);
        return new Module(namespace, resource, sourceHash, forms, buildSchemaIndex(namespace, forms), origin);
      }
    } catch (EOFException error) {
      throw invalid("truncated artifact");
    } catch (IOException error) {
      throw invalid(error.getMessage());
    }
  }

  static String declaredNamespace(Object[] forms) {
    for (Object form : forms) {
      if (!(form instanceof hara.lang.data.List<?> list) || list.count() < 2) continue;
      if (!(list.nth(0) instanceof Symbol operator)
          || operator.getNamespace() != null
          || !"ns".equals(operator.getName())) continue;
      if (!(list.nth(1) instanceof Symbol namespace)
          || namespace.getNamespace() != null) {
        throw new HaraException("HALC source has an invalid ns declaration");
      }
      return namespace.getName();
    }
    throw new HaraException("HALC source does not declare a namespace");
  }

  @SuppressWarnings("unchecked")
  private static Object[] canonicalizeSchemaReferences(String namespace, Object[] forms) {
    Set<String> definitions = new HashSet<>();
    Map<String, Integer> schemaValues = new HashMap<>();
    Map<String, String> aliases = namespaceAliases(forms);
    for (int index = 0; index < forms.length; index++) {
      Object form = forms[index];
      if (!(form instanceof hara.lang.data.List<?> list) || list.count() < 2) continue;
      if (!(list.nth(0) instanceof Symbol operator) || operator.getNamespace() != null) continue;
      if (!(list.nth(1) instanceof Symbol name) || name.getNamespace() != null) continue;
      if (Set.of("def", "defn", "defn-", "defmacro", "defstruct", "defmutable", "declare")
          .contains(operator.getName())) {
        definitions.add(name.getName());
      }
      if ("def".equals(operator.getName()) && list.count() >= 3) {
        schemaValues.put(name.getName(), index);
      }
    }

    Object[] canonical = forms.clone();
    Deque<String> schemaRoots = new ArrayDeque<>();
    for (int index = 0; index < canonical.length; index++) {
      Object form = canonical[index];
      if (!(form instanceof hara.lang.data.List<?> list) || list.count() < 2) continue;
      if (!(list.nth(0) instanceof Symbol operator)
          || operator.getNamespace() != null
          || !("defn".equals(operator.getName()) || "defn-".equals(operator.getName()))) {
        continue;
      }
      if (!(list.nth(1) instanceof Symbol name)
          || !(name.meta() instanceof IMapType<?, ?> rawMetadata)) {
        continue;
      }
      IMapType<Object, Object> metadata = (IMapType<Object, Object>) rawMetadata;
      Object schema = metadata.lookup(Keyword.create("schema"));
      if (!(schema instanceof hara.lang.data.List<?> reference)
          || reference.count() != 2
          || !(reference.nth(0) instanceof Symbol varOperator)
          || varOperator.getNamespace() != null
          || !"var".equals(varOperator.getName())
          || !(reference.nth(1) instanceof Symbol target)) {
        continue;
      }
      String targetNamespace = target.getNamespace();
      if (targetNamespace == null || "-".equals(targetNamespace)) {
        targetNamespace = namespace;
      } else {
        targetNamespace = aliases.getOrDefault(targetNamespace, targetNamespace);
      }
      if (namespace.equals(targetNamespace) && !definitions.contains(target.getName())) {
        throw new HaraException("schema Var does not exist: " + target.display());
      }
      if (namespace.equals(targetNamespace)) schemaRoots.add(target.getName());
      Symbol qualified = Symbol.create(targetNamespace, target.getName());
      Object qualifiedReference =
          hara.lang.data.List.Standard.from(
              reference.meta(), new Object[] {reference.nth(0), qualified});
      IMapType<Object, Object> qualifiedMetadata =
          (IMapType<Object, Object>) metadata.assoc(Keyword.create("schema"), qualifiedReference);
      Object[] values = new Object[Math.toIntExact(list.count())];
      for (int item = 0; item < values.length; item++) values[item] = list.nth(item);
      values[1] = name.withMeta(qualifiedMetadata);
      canonical[index] = hara.lang.data.List.Standard.from(list.meta(), values);
    }

    Set<String> visited = new HashSet<>();
    while (!schemaRoots.isEmpty()) {
      String schemaName = schemaRoots.removeFirst();
      if (!visited.add(schemaName)) continue;
      Integer index = schemaValues.get(schemaName);
      if (index == null) continue;
      hara.lang.data.List<?> definition = (hara.lang.data.List<?>) canonical[index];
      Object[] values = new Object[Math.toIntExact(definition.count())];
      for (int item = 0; item < values.length; item++) values[item] = definition.nth(item);
      values[2] =
          canonicalizeNestedSchemaReferences(
              values[2], namespace, aliases, definitions, schemaRoots);
      canonical[index] = hara.lang.data.List.Standard.from(definition.meta(), values);
    }
    return canonical;
  }

  @SuppressWarnings("unchecked")
  private static SchemaIndex buildSchemaIndex(String namespace, Object[] forms) {
    Map<String, Object> values = new HashMap<>();
    Map<String, Object> functions = new HashMap<>();
    Deque<String> roots = new ArrayDeque<>();
    for (Object form : forms) {
      if (!(form instanceof hara.lang.data.List<?> definition) || definition.count() < 2) continue;
      if (!(definition.nth(0) instanceof Symbol operator) || operator.getNamespace() != null) continue;
      Object binding = definition.nth(1);
      Symbol name = binding instanceof Symbol symbol
          ? symbol
          : binding instanceof IObjType object && object instanceof Symbol symbol ? symbol : null;
      if (name == null) continue;
      if ("def".equals(operator.getName()) && definition.count() >= 3) {
        values.put(name.getName(), definition.nth(2));
        continue;
      }
      if (("defstruct".equals(operator.getName()) || "defmutable".equals(operator.getName()))
          && definition.count() >= 3
          && definition.nth(2) instanceof ILinearType<?> fieldValues) {
        HalcSchema.NamedField[] fields =
            new HalcSchema.NamedField[Math.toIntExact(fieldValues.count())];
        Set<String> seen = new HashSet<>();
        for (int index = 0; index < fields.length; index++) {
          try {
            fields[index] = HalcSchema.normalizeNamedField(fieldValues.nth(index));
          } catch (HaraException error) {
            throw new HaraException(
                "invalid " + operator.getName() + " field: " + error.getMessage());
          }
          if (!seen.add(fields[index].name())) {
            throw new HaraException(
                "Duplicate " + operator.getName() + " field: " + fields[index].name());
          }
        }
        values.put(
            name.getName(),
            HalcSchema.namedTypeSchema(
                namespace + "/" + name.getName(),
                "defmutable".equals(operator.getName()),
                fields));
        continue;
      }
      if (!("defn".equals(operator.getName()) || "defn-".equals(operator.getName()))
          || !(name.meta() instanceof IMapType<?, ?> rawMetadata)) continue;
      Object schema = ((IMapType<Object, Object>) rawMetadata).lookup(Keyword.create("schema"));
      if (schema == null) continue;
      functions.put(namespace + "/" + name.getName(), schema);
      collectLocalSchemaReferences(schema, namespace, roots);
    }

    Map<String, Object> definitions = new HashMap<>();
    Set<String> visited = new HashSet<>();
    while (!roots.isEmpty()) {
      String name = roots.removeFirst();
      if (!visited.add(name)) continue;
      Object value = values.get(name);
      if (value == null) continue;
      definitions.put(namespace + "/" + name, value);
      collectLocalSchemaReferences(value, namespace, roots);
    }
    Map<String, HalcSchema.Type> definitionTypes = new HashMap<>();
    for (Entry<String, Object> entry : definitions.entrySet()) {
      try {
        definitionTypes.put(entry.getKey(), HalcSchema.normalize(entry.getValue()));
      } catch (HaraException error) {
        throw new HaraException("invalid schema " + entry.getKey() + ": " + error.getMessage());
      }
    }
    Map<String, HalcSchema.Type> functionTypes = new HashMap<>();
    for (Entry<String, Object> entry : functions.entrySet()) {
      try {
        functionTypes.put(entry.getKey(), HalcSchema.normalize(entry.getValue()));
      } catch (HaraException error) {
        throw new HaraException(
            "invalid function schema " + entry.getKey() + ": " + error.getMessage());
      }
    }
    Map<String, HalcSchema.Type> inferredFunctionTypes =
        HalcSchema.inferFunctionTypes(namespace, forms, functionTypes, definitionTypes);
    return new SchemaIndex(
        definitions, functions, definitionTypes, functionTypes, inferredFunctionTypes);
  }

  private static void collectLocalSchemaReferences(
      Object value, String namespace, Deque<String> output) {
    if (value instanceof hara.lang.data.List<?> reference
        && reference.count() == 2
        && reference.nth(0) instanceof Symbol operator
        && operator.getNamespace() == null
        && "var".equals(operator.getName())
        && reference.nth(1) instanceof Symbol target) {
      if (namespace.equals(target.getNamespace())) output.add(target.getName());
      return;
    }
    if (value instanceof ILinearType<?> values) {
      for (int index = 0; index < values.count(); index++) {
        collectLocalSchemaReferences(values.nth(index), namespace, output);
      }
    } else if (value instanceof IMapType<?, ?> map) {
      for (Object item : map) {
        Entry<?, ?> entry = (Entry<?, ?>) item;
        collectLocalSchemaReferences(entry.getKey(), namespace, output);
        collectLocalSchemaReferences(entry.getValue(), namespace, output);
      }
    } else if (value instanceof ISetType<?> set) {
      for (Object item : set) collectLocalSchemaReferences(item, namespace, output);
    }
  }

  private static Object canonicalizeNestedSchemaReferences(
      Object value,
      String namespace,
      Map<String, String> aliases,
      Set<String> definitions,
      Deque<String> localReferences) {
    if (value instanceof hara.lang.data.List<?> reference
        && reference.count() == 2
        && reference.nth(0) instanceof Symbol operator
        && operator.getNamespace() == null
        && "var".equals(operator.getName())
        && reference.nth(1) instanceof Symbol target) {
      String targetNamespace = target.getNamespace();
      if (targetNamespace == null || "-".equals(targetNamespace)) {
        targetNamespace = namespace;
      } else {
        targetNamespace = aliases.getOrDefault(targetNamespace, targetNamespace);
      }
      if (namespace.equals(targetNamespace)) {
        if (!definitions.contains(target.getName())) {
          throw new HaraException("schema Var does not exist: " + target.display());
        }
        localReferences.add(target.getName());
      }
      return hara.lang.data.List.Standard.from(
          reference.meta(),
          new Object[] {reference.nth(0), Symbol.create(targetNamespace, target.getName())});
    }
    if (value instanceof hara.lang.data.List<?> list) {
      return hara.lang.data.List.Standard.from(list.meta(), canonicalizeLinear(list, namespace, aliases, definitions, localReferences));
    }
    if (value instanceof ILinearType<?> vector && "[".equals(vector.startString())) {
      Object[] canonical =
          canonicalizeLinear(vector, namespace, aliases, definitions, localReferences);
      Object sequence =
          canonical.length <= 8
              ? hara.kernel.builtin.BuiltinStruct.tuple(canonical)
              : hara.lang.data.Vector.Standard.from(null, canonical);
      return ((IObjType) sequence).withMeta(((IObjType) vector).meta());
    }
    if (value instanceof IMapType<?, ?> map) {
      Object[] entries = new Object[Math.toIntExact(map.count() * 2)];
      int index = 0;
      for (Object item : map) {
        Entry<?, ?> entry = (Entry<?, ?>) item;
        entries[index++] = canonicalizeNestedSchemaReferences(entry.getKey(), namespace, aliases, definitions, localReferences);
        entries[index++] = canonicalizeNestedSchemaReferences(entry.getValue(), namespace, aliases, definitions, localReferences);
      }
      IMetadata metadata = ((IObjType) map).meta();
      return value instanceof hara.lang.data.OrderedMap<?, ?>
          ? hara.lang.data.OrderedMap.Standard.from(metadata, entries)
          : hara.lang.data.Map.Standard.from(metadata, entries);
    }
    if (value instanceof ISetType<?> set) {
      Object[] elements = new Object[Math.toIntExact(set.count())];
      int index = 0;
      for (Object element : set) {
        elements[index++] = canonicalizeNestedSchemaReferences(element, namespace, aliases, definitions, localReferences);
      }
      IMetadata metadata = ((IObjType) set).meta();
      return value instanceof hara.lang.data.OrderedSet<?>
          ? hara.lang.data.OrderedSet.Standard.from(metadata, elements)
          : hara.lang.data.Set.Standard.from(metadata, elements);
    }
    return value;
  }

  private static Object[] canonicalizeLinear(
      ILinearType<?> values,
      String namespace,
      Map<String, String> aliases,
      Set<String> definitions,
      Deque<String> localReferences) {
    Object[] canonical = new Object[Math.toIntExact(values.count())];
    for (int index = 0; index < canonical.length; index++) {
      canonical[index] = canonicalizeNestedSchemaReferences(values.nth(index), namespace, aliases, definitions, localReferences);
    }
    return canonical;
  }

  private static Map<String, String> namespaceAliases(Object[] forms) {
    Map<String, String> aliases = new HashMap<>();
    for (Object form : forms) {
      if (!(form instanceof hara.lang.data.List<?> declaration) || declaration.count() < 2) {
        continue;
      }
      if (!(declaration.nth(0) instanceof Symbol operator)
          || operator.getNamespace() != null
          || !"ns".equals(operator.getName())) {
        continue;
      }
      for (int clauseIndex = 2; clauseIndex < declaration.count(); clauseIndex++) {
        if (!(declaration.nth(clauseIndex) instanceof hara.lang.data.List<?> clause)
            || clause.count() < 2
            || !(clause.nth(0) instanceof Keyword keyword)
            || !"require".equals(keyword.getName())) {
          continue;
        }
        for (int specIndex = 1; specIndex < clause.count(); specIndex++) {
          if (!(clause.nth(specIndex) instanceof ILinearType<?> spec)
              || !"[".equals(spec.startString())
              || spec.count() < 3
              || !(spec.nth(0) instanceof Symbol target)) {
            continue;
          }
          for (int option = 1; option + 1 < spec.count(); option += 2) {
            if (spec.nth(option) instanceof Keyword key
                && "as".equals(key.getName())
                && spec.nth(option + 1) instanceof Symbol alias) {
              aliases.put(alias.getName(), target.display());
            }
          }
        }
      }
    }
    return aliases;
  }

  private static void writeValue(DataOutputStream output, Object value) throws IOException {
    if (value == null) {
      output.writeByte(NIL);
    } else if (Boolean.FALSE.equals(value)) {
      output.writeByte(FALSE);
    } else if (Boolean.TRUE.equals(value)) {
      output.writeByte(TRUE);
    } else if (value instanceof Byte
        || value instanceof Short
        || value instanceof Integer
        || value instanceof Long) {
      output.writeByte(LONG);
      output.writeLong(((Number) value).longValue());
    } else if (value instanceof Float || value instanceof Double) {
      output.writeByte(DOUBLE);
      output.writeDouble(((Number) value).doubleValue());
    } else if (value instanceof BigInteger number) {
      Number normalized = hara.lang.base.NumUtils.normalizeInteger(number);
      if (normalized instanceof Long integer) {
        output.writeByte(LONG);
        output.writeLong(integer);
      } else {
        output.writeByte(BIG_INTEGER);
        writeString(output, normalized.toString());
      }
    } else if (value instanceof String string) {
      output.writeByte(STRING);
      writeString(output, string);
    } else if (value instanceof HaraCharacter character) {
      output.writeByte(CHARACTER);
      output.writeInt(character.codePoint());
    } else if (value instanceof Character character) {
      output.writeByte(CHARACTER);
      output.writeInt(character);
    } else if (value instanceof Symbol symbol) {
      output.writeByte(SYMBOL);
      writeNamespaced(output, symbol.getNamespace(), symbol.getName());
      writeMetadata(output, symbol);
    } else if (value instanceof Keyword keyword) {
      output.writeByte(KEYWORD);
      writeNamespaced(output, keyword.getNamespace(), keyword.getName());
      writeMetadata(output, keyword);
    } else if (value instanceof hara.lang.data.List<?> list) {
      output.writeByte(LIST);
      writeLinear(output, list);
      writeMetadata(output, list);
    } else if (value instanceof hara.lang.data.Tuple.Tup0
        || value instanceof hara.lang.data.Tuple.Tup1<?>) {
      output.writeByte(VECTOR);
      writeLinear(output, (ILinearType<?>) value);
      writeMetadata(output, (IObjType) value);
    } else if (value instanceof hara.lang.data.Vector<?> vector) {
      output.writeByte(VECTOR);
      writeLinear(output, vector);
      writeMetadata(output, vector);
    } else if (value instanceof hara.lang.data.OrderedMap<?, ?> map) {
      output.writeByte(ORDERED_MAP);
      writeMap(output, map, false);
      writeMetadata(output, map);
    } else if (value instanceof IMapType<?, ?> map) {
      output.writeByte(MAP);
      writeMap(output, map, true);
      writeMetadata(output, (IObjType) map);
    } else if (value instanceof hara.lang.data.OrderedSet<?> set) {
      output.writeByte(ORDERED_SET);
      writeSet(output, set, false);
      writeMetadata(output, set);
    } else if (value instanceof ISetType<?> set) {
      output.writeByte(SET);
      writeSet(output, set, true);
      writeMetadata(output, (IObjType) set);
    } else if (value instanceof java.util.regex.Pattern pattern) {
      if (pattern.flags() != 0) {
        throw new HaraException(
            "Unsupported portable HALC regex flags: " + pattern.flags() + " for " + pattern);
      }
      output.writeByte(REGEX);
      writeString(output, pattern.pattern());
    } else {
      throw new HaraException(
          "Unsupported portable HALC constant: " + value.getClass().getName());
    }
  }

  private static Object readValue(DataInputStream input) throws IOException {
    int opcode = input.readUnsignedByte();
    return switch (opcode) {
      case NIL -> null;
      case FALSE -> Boolean.FALSE;
      case TRUE -> Boolean.TRUE;
      case LONG -> input.readLong();
      case DOUBLE -> input.readDouble();
      case BIG_INTEGER -> hara.lang.base.NumUtils.normalizeInteger(new BigInteger(readString(input)));
      case STRING -> readString(input);
      case CHARACTER -> HaraCharacter.of(input.readInt());
      case SYMBOL ->
          withMetadata(
              Symbol.create(readNullableString(input), readString(input)), readMetadata(input));
      case KEYWORD ->
          withMetadata(
              Keyword.create(readNullableString(input), readString(input)), readMetadata(input));
      case LIST -> {
        Object[] values = readValues(input);
        yield withMetadata(hara.lang.data.List.Standard.from(null, values), readMetadata(input));
      }
      case VECTOR -> {
        Object[] values = readValues(input);
        IMetadata metadata = readMetadata(input);
        Object sequence =
            values.length <= 8
                ? hara.kernel.builtin.BuiltinStruct.tuple(values)
                : hara.lang.data.Vector.Standard.from(null, values);
        yield withMetadata((IObjType) sequence, metadata);
      }
      case MAP -> {
        Object[] entries = readEntries(input);
        yield withMetadata(hara.lang.data.Map.Standard.from(null, entries), readMetadata(input));
      }
      case ORDERED_MAP -> {
        Object[] entries = readEntries(input);
        yield withMetadata(
            hara.lang.data.OrderedMap.Standard.from(null, entries), readMetadata(input));
      }
      case SET -> {
        Object[] values = readValues(input);
        yield withMetadata(hara.lang.data.Set.Standard.from(null, values), readMetadata(input));
      }
      case ORDERED_SET -> {
        Object[] values = readValues(input);
        yield withMetadata(
            hara.lang.data.OrderedSet.Standard.from(null, values), readMetadata(input));
      }
      case REGEX -> java.util.regex.Pattern.compile(readString(input));
      default -> throw invalid("unknown value opcode");
    };
  }

  private static void writeLinear(DataOutputStream output, ILinearType<?> values)
      throws IOException {
    writeCount(output, Math.toIntExact(values.count()));
    for (Object value : values) writeValue(output, value);
  }

  private static Object[] readValues(DataInputStream input) throws IOException {
    int count = readCount(input);
    Object[] values = new Object[count];
    for (int index = 0; index < count; index++) values[index] = readValue(input);
    return values;
  }

  private static void writeMap(DataOutputStream output, IMapType<?, ?> map, boolean canonical)
      throws IOException {
    writeCount(output, Math.toIntExact(map.count()));
    int count = Math.toIntExact(map.count());
    if (!canonical) {
      // Ordered maps: entry order is semantic, keep iteration order.
      for (Object item : map) {
        Entry<?, ?> entry = (Entry<?, ?>) item;
        writeValue(output, entry.getKey());
        writeValue(output, entry.getValue());
      }
      return;
    }
    byte[][] encodedKeys = new byte[count][];
    byte[][] encodedEntries = new byte[count][];
    int index = 0;
    for (Object item : map) {
      Entry<?, ?> entry = (Entry<?, ?>) item;
      encodedKeys[index] = encodeValue(entry.getKey());
      encodedEntries[index] = concat(encodedKeys[index], encodeValue(entry.getValue()));
      index++;
    }
    for (int order : sortedOrder(encodedKeys)) output.write(encodedEntries[order]);
  }

  private static Object[] readEntries(DataInputStream input) throws IOException {
    int count = readCount(input);
    Object[] entries = new Object[Math.multiplyExact(count, 2)];
    for (int index = 0; index < entries.length; index++) entries[index] = readValue(input);
    return entries;
  }

  private static void writeSet(DataOutputStream output, ISetType<?> set, boolean canonical)
      throws IOException {
    writeCount(output, Math.toIntExact(set.count()));
    if (!canonical) {
      // Ordered sets: element order is semantic, keep iteration order.
      for (Object value : set) writeValue(output, value);
      return;
    }
    byte[][] encodedValues = new byte[Math.toIntExact(set.count())][];
    int index = 0;
    for (Object value : set) encodedValues[index++] = encodeValue(value);
    for (int order : sortedOrder(encodedValues)) output.write(encodedValues[order]);
  }

  // Canonical collection ordering: entries are sorted by the unsigned
  // lexicographic order of their canonical encodings, so the artifact does
  // not depend on any host map/set iteration order.
  private static byte[] encodeValue(Object value) throws IOException {
    ByteArrayOutputStream bytes = new ByteArrayOutputStream();
    try (DataOutputStream output = new DataOutputStream(bytes)) {
      writeValue(output, value);
    }
    return bytes.toByteArray();
  }

  private static byte[] concat(byte[] first, byte[] second) {
    byte[] both = Arrays.copyOf(first, first.length + second.length);
    System.arraycopy(second, 0, both, first.length, second.length);
    return both;
  }

  private static int[] sortedOrder(byte[][] encoded) {
    Integer[] order = new Integer[encoded.length];
    for (int index = 0; index < order.length; index++) order[index] = index;
    Arrays.sort(order, (a, b) -> Arrays.compareUnsigned(encoded[a], encoded[b]));
    int[] result = new int[order.length];
    for (int index = 0; index < order.length; index++) result[index] = order[index];
    return result;
  }

  private static void writeNamespaced(DataOutputStream output, String namespace, String name)
      throws IOException {
    writeNullableString(output, namespace);
    writeString(output, name);
  }

  private static void writeMetadata(DataOutputStream output, IObjType value) throws IOException {
    IMetadata metadata = value.meta();
    if (metadata instanceof IMapType<?, ?> map) {
      Object portable = map;
      for (String key : new String[] {"line", "column", "end-line", "end-column", "file"}) {
        portable = ((IMapType) portable).dissoc(Keyword.create(key));
      }
      metadata = (IMetadata) portable;
      if (((IMapType<?, ?>) metadata).count() == 0) metadata = null;
    }
    if (metadata == null) {
      output.writeBoolean(false);
    } else if (metadata instanceof IMapType<?, ?>) {
      output.writeBoolean(true);
      writeValue(output, metadata);
    } else {
      throw new HaraException(
          "Unsupported portable HALC metadata: " + metadata.getClass().getName());
    }
  }

  private static IMetadata readMetadata(DataInputStream input) throws IOException {
    if (!input.readBoolean()) return null;
    Object metadata = readValue(input);
    if (!(metadata instanceof IMetadata)) throw invalid("metadata is not metadata-capable");
    return (IMetadata) metadata;
  }

  @SuppressWarnings("unchecked")
  private static <T extends IObjType> T withMetadata(T value, IMetadata metadata) {
    return metadata == null ? value : (T) value.withMeta(metadata);
  }

  private static void writeString(DataOutputStream output, String value) throws IOException {
    byte[] bytes = value.getBytes(StandardCharsets.UTF_8);
    output.writeInt(bytes.length);
    output.write(bytes);
  }

  private static String readString(DataInputStream input) throws IOException {
    int length = input.readInt();
    if (length < 0 || length > MAX_PAYLOAD_BYTES) throw invalid("invalid string length " + length);
    byte[] bytes = input.readNBytes(length);
    if (bytes.length != length) throw invalid("truncated string");
    return new String(bytes, StandardCharsets.UTF_8);
  }

  private static void writeNullableString(DataOutputStream output, String value)
      throws IOException {
    output.writeBoolean(value != null);
    if (value != null) writeString(output, value);
  }

  private static String readNullableString(DataInputStream input) throws IOException {
    return input.readBoolean() ? readString(input) : null;
  }

  private static void writeCount(DataOutputStream output, int count) throws IOException {
    if (count < 0 || count > MAX_COLLECTION_ITEMS) {
      throw new HaraException("HALC collection is too large: " + count);
    }
    output.writeInt(count);
  }

  private static int readCount(DataInputStream input) throws IOException {
    int count = input.readInt();
    if (count < 0 || count > MAX_COLLECTION_ITEMS) {
      throw invalid("invalid collection count " + count);
    }
    return count;
  }

  private static byte[] sha256(byte[] value) {
    try {
      return MessageDigest.getInstance("SHA-256").digest(value);
    } catch (NoSuchAlgorithmException impossible) {
      throw new AssertionError(impossible);
    }
  }

  private static HaraException invalid(String detail) {
    return new HaraException("Invalid HALC artifact: " + detail);
  }

  static final class Module {
    final String namespace;
    final String resource;
    final byte[] sourceHash;
    final Object[] forms;
    final SchemaIndex schemas;
    final Origin origin;

    Module(
        String namespace,
        String resource,
        byte[] sourceHash,
        Object[] forms,
        SchemaIndex schemas,
        Origin origin) {
      this.namespace = namespace;
      this.resource = resource;
      this.sourceHash = sourceHash.clone();
      this.forms = forms.clone();
      this.schemas = schemas;
      this.origin = origin;
    }
  }

  static final class SchemaIndex {
    final Map<String, Object> definitions;
    final Map<String, Object> functions;
    final Map<String, HalcSchema.Type> definitionTypes;
    final Map<String, HalcSchema.Type> functionTypes;
    final Map<String, HalcSchema.Type> inferredFunctionTypes;

    SchemaIndex(
        Map<String, Object> definitions,
        Map<String, Object> functions,
        Map<String, HalcSchema.Type> definitionTypes,
        Map<String, HalcSchema.Type> functionTypes,
        Map<String, HalcSchema.Type> inferredFunctionTypes) {
      this.definitions = Map.copyOf(definitions);
      this.functions = Map.copyOf(functions);
      this.definitionTypes = Map.copyOf(definitionTypes);
      this.functionTypes = Map.copyOf(functionTypes);
      this.inferredFunctionTypes = Map.copyOf(inferredFunctionTypes);
    }

    HalcSchema.Type resolvedFunctionType(String qualifiedVar) {
      HalcSchema.Type schema = functionTypes.get(qualifiedVar);
      if (schema instanceof HalcSchema.Reference reference) {
        return definitionTypes.getOrDefault(reference.name(), schema);
      }
      return schema;
    }
  }

  enum Origin {
    HALC,
    LEGACY_HIR
  }
}
