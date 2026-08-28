package hara.truffle;

import hara.lang.base.G;
import hara.lang.base.NumUtils;
import hara.lang.data.Keyword;
import hara.lang.data.Symbol;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.kernel.builtin.BuiltinStruct;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Map.Entry;

/** Compiler-facing normalization of portable HAL schema forms. */
public final class HalcSchema {
  private HalcSchema() {}

  public sealed interface Type
      permits Primitive,
          Reference,
          Union,
          VectorType,
          SetType,
          Tuple,
          MapType,
          StructType,
          Properties,
          FunctionType,
          EnumType,
          Extension,
          Unknown {}

  public record Primitive(String name) implements Type {}

  public record Reference(String name) implements Type {}

  public record Union(List<Type> types) implements Type {
    public Union {
      types = List.copyOf(types);
    }
  }

  public record VectorType(Type item) implements Type {}

  public record SetType(Type item) implements Type {}

  public record Tuple(List<Type> items) implements Type {
    public Tuple {
      items = List.copyOf(items);
    }
  }

  public record Field(Object name, Object properties, Type type) {}

  public record MapType(List<Field> fields) implements Type {
    public MapType {
      fields = List.copyOf(fields);
    }
  }

  public record StructType(String name, boolean mutable, List<Field> fields) implements Type {
    public StructType {
      fields = List.copyOf(fields);
    }
  }

  /** A named-value declaration field before it is published as a runtime descriptor. */
  public record NamedField(String name, Object properties, Object schema, Type type) {}

  public record Properties(Type schema, Object properties) implements Type {}

  public record Function(List<Type> fixed, Type rest, Type output) {
    public Function {
      fixed = List.copyOf(fixed);
    }
  }

  public record FunctionType(List<Function> arities) implements Type {
    public FunctionType {
      arities = List.copyOf(arities);
    }
  }

  public record EnumType(List<Object> values) implements Type {
    public EnumType {
      values = List.copyOf(values);
    }
  }

  public record Extension(String head, List<Object> arguments) implements Type {
    public Extension {
      arguments = List.copyOf(arguments);
    }
  }

  public record Unknown(Object surface) implements Type {}

  public static Object shorthand(Type schema) {
    if (schema instanceof Properties decorated) {
      ILinearType<?> surface = vector(shorthand(decorated.schema()));
      if (surface == null || surface.count() == 0) return shorthand(decorated.schema());
      ArrayList<Object> values = new ArrayList<>();
      values.add(surface.nth(0));
      values.add(decorated.properties());
      for (int index = 1; index < surface.count(); index++) values.add(surface.nth(index));
      return vectorOf(values.toArray());
    }
    if (schema instanceof Primitive primitive) {
      return vectorOf(Keyword.create(primitive.name()));
    }
    if (schema instanceof Reference reference) {
      return vectorOf(
          hara.lang.data.List.Standard.from(
              null, Symbol.create("var"), Symbol.create(reference.name())));
    }
    if (schema instanceof Union union) {
      ArrayList<Object> values = headed("or");
      union.types().forEach(value -> values.add(shorthand(value)));
      return vectorOf(values.toArray());
    }
    if (schema instanceof VectorType vector) {
      return vectorOf(Keyword.create("vector"), shorthand(vector.item()));
    }
    if (schema instanceof SetType set) {
      return vectorOf(Keyword.create("set"), shorthand(set.item()));
    }
    if (schema instanceof Tuple tuple) {
      ArrayList<Object> values = headed("tuple");
      tuple.items().forEach(value -> values.add(shorthand(value)));
      return vectorOf(values.toArray());
    }
    if (schema instanceof MapType map) {
      ArrayList<Object> values = headed("map");
      map.fields().forEach(
          field -> {
            if (field.properties() == null)
              values.add(vectorOf(field.name(), shorthand(field.type())));
            else
              values.add(vectorOf(field.name(), field.properties(), shorthand(field.type())));
          });
      return vectorOf(values.toArray());
    }
    if (schema instanceof StructType struct) {
      ArrayList<Object> values = headed("struct");
      if (struct.mutable()) {
        values.add(
            hara.lang.data.Map.Standard.from(
                null, Keyword.create("mutable?"), Boolean.TRUE));
      }
      values.add(
          hara.lang.data.List.Standard.from(
              null, Symbol.create("var"), Symbol.create(struct.name())));
      struct.fields().forEach(
          field -> {
            if (field.properties() == null)
              values.add(vectorOf(field.name(), shorthand(field.type())));
            else
              values.add(vectorOf(field.name(), field.properties(), shorthand(field.type())));
          });
      return vectorOf(values.toArray());
    }
    if (schema instanceof FunctionType function) {
      if (function.arities().size() == 1) return functionShorthand(function.arities().get(0));
      ArrayList<Object> values = headed("function");
      function.arities().forEach(arity -> values.add(functionShorthand(arity)));
      return vectorOf(values.toArray());
    }
    if (schema instanceof EnumType enumeration) {
      ArrayList<Object> values = headed("enum");
      values.addAll(enumeration.values());
      return vectorOf(values.toArray());
    }
    if (schema instanceof Extension extension) {
      ArrayList<Object> values = headed(extension.head());
      values.addAll(extension.arguments());
      return vectorOf(values.toArray());
    }
    Object surface = ((Unknown) schema).surface();
    return vector(surface) == null ? vectorOf(surface) : surface;
  }

  private static Object functionShorthand(Function function) {
    ArrayList<Object> inputs = new ArrayList<>();
    function.fixed().forEach(value -> inputs.add(shorthand(value)));
    if (function.rest() != null) {
      inputs.add(Symbol.create("&"));
      inputs.add(shorthand(function.rest()));
    }
    return vectorOf(
        Keyword.create("fn"), vectorOf(inputs.toArray()), shorthand(function.output()));
  }

  private static ArrayList<Object> headed(String head) {
    ArrayList<Object> values = new ArrayList<>();
    values.add(Keyword.create(head));
    return values;
  }

  private static Object vectorOf(Object... values) {
    return BuiltinStruct.vector(values);
  }

  public static Type normalize(Object schema) {
    if (schema instanceof Keyword keyword) {
      String name = keywordName(keyword);
      return "integer".equals(name) ? integerType() : new Primitive(name);
    }
    if (schema instanceof hara.lang.protocol.IMapType<?, ?> map) {
      @SuppressWarnings("unchecked")
      hara.lang.protocol.IMapType<Object, Object> valuesMap =
          (hara.lang.protocol.IMapType<Object, Object>) map;
      Object kindValue = valuesMap.lookup(Keyword.create("kind"));
      if (kindValue instanceof Keyword kind) {
        return normalizeLonghand(valuesMap, kind.getName());
      }
    }    if (schema instanceof hara.lang.data.List<?> reference
        && reference.count() == 2
        && reference.nth(0) instanceof Symbol operator
        && operator.getNamespace() == null
        && "var".equals(operator.getName())) {
      if (!(reference.nth(1) instanceof Symbol target)) {
        throw invalid("named schema reference must target a symbol");
      }
      if (target.getNamespace() == null) {
        throw invalid("named schema reference is not fully qualified: " + target.display());
      }
      return new Reference(target.display());
    }
    ILinearType<?> vector = vector(schema);
    if (vector == null || vector.count() == 0) {
      return new Unknown(schema);
    }
    if (!(vector.nth(0) instanceof Keyword head)) return new Unknown(schema);
    List<Object> arguments = values(vector, 1);
    String headName = keywordName(head);
    Object properties = null;
    if (supportsProperties(headName) && !arguments.isEmpty() && schemaMap(arguments.get(0)) != null) {
      properties = arguments.remove(0);
    }
    Type normalized = switch (headName) {
      case "or" -> normalizeUnion(arguments);
      case "maybe" -> {
        requireCount(headName, arguments, 1);
        yield normalizeUnion(List.of(arguments.get(0), Keyword.create("nil")));
      }
      case "vector" -> {
        requireCount(headName, arguments, 1);
        yield new VectorType(normalize(arguments.get(0)));
      }
      case "set" -> {
        requireCount(headName, arguments, 1);
        yield new SetType(normalize(arguments.get(0)));
      }
      case "tuple" -> new Tuple(normalizeAll(arguments));
      case "map" -> normalizeMap(arguments);
      case "struct" -> {
        boolean mutable = false;
        List<Object> structArguments = arguments;
        if (!arguments.isEmpty() && schemaMap(arguments.get(0)) != null) {
          IMapType<Object, Object> structProperties = schemaMap(arguments.get(0));
          Object mutableValue = longhandValue(structProperties, "mutable?");
          if (mutableValue != null) {
            if (!(mutableValue instanceof Boolean value)) {
              throw invalid("struct schema :mutable? must be boolean");
            }
            mutable = value;
            structArguments = arguments.subList(1, arguments.size());
          }
        }
        yield normalizeStructForms(structArguments, mutable);
      }
      case "fn" -> new FunctionType(List.of(normalizeFunction(vector)));
      case "function" -> {
        if (arguments.isEmpty()) {
          throw invalid(":function schema requires at least one :fn schema");
        }
        List<Function> arities = new ArrayList<>();
        for (Object argument : arguments) {
          ILinearType<?> function = vector(argument);
          if (function == null) {
            throw invalid(":function members must be :fn schemas");
          }
          arities.add(normalizeFunction(function));
        }
        yield new FunctionType(arities);
      }
      case "enum" -> new EnumType(arguments);
      case "integer" -> {
        if (!arguments.isEmpty()) yield new Extension(headName, arguments);
        yield integerType();
      }
      default -> arguments.isEmpty()
          ? new Primitive(headName)
          : new Extension(headName, arguments);
    };
    return properties == null ? normalized : new Properties(normalized, properties);
  }

  private static boolean supportsProperties(String head) {
    return List.of(
            "str",
            "string",
            "keyword",
            "symbol",
            "list",
            "bytes",
            "int",
            "long",
            "bigint",
            "integer",
            "num",
            "number",
            "any",
            "vector",
            "set",
            "map")
        .contains(head);
  }

  private static Entry<Object, Object> longhandEntry(
      IMapType<Object, Object> schema, String name) {
    return schema.find(Keyword.create(name));
  }

  private static Object longhandValue(IMapType<Object, Object> schema, String name) {
    Entry<Object, Object> entry = longhandEntry(schema, name);
    return entry == null ? null : entry.getValue();
  }

  private static List<Object> longhandValues(
      IMapType<Object, Object> schema, String name, List<Object> fallback) {
    Entry<Object, Object> entry = longhandEntry(schema, name);
    if (entry == null) return fallback;
    ILinearType<?> values = vector(entry.getValue());
    if (values == null) throw invalid("schema :" + name + " must be a vector");
    return values(values, 0);
  }

  @SuppressWarnings("unchecked")
  private static IMapType<Object, Object> schemaMap(Object value) {
    return value instanceof IMapType<?, ?> map
        ? (IMapType<Object, Object>) map
        : null;
  }

  private static String keywordName(Keyword keyword) {
    return keyword.getNamespace() == null
        ? keyword.getName()
        : keyword.getNamespace() + "/" + keyword.getName();
  }

  private static Reference normalizeReference(Object value) {
    if (!(value instanceof Symbol name)) {
      throw invalid("named schema reference must target a symbol");
    }
    if (name.getNamespace() == null) {
      throw invalid("named schema reference is not fully qualified: " + name.display());
    }
    return new Reference(name.display());
  }

  private static String normalizeStructName(Object value) {
    if (value instanceof hara.lang.data.List<?> reference
        && reference.count() == 2
        && reference.nth(0) instanceof Symbol operator
        && operator.getNamespace() == null
        && "var".equals(operator.getName())) {
      if (!(reference.nth(1) instanceof Symbol target)) {
        throw invalid("named struct schema reference must target a symbol");
      }
      if (target.getNamespace() == null) {
        throw invalid("named struct schema reference is not fully qualified: " + target.display());
      }
      return target.display();
    }
    if (value instanceof Symbol symbol) {
      if (symbol.getNamespace() == null) {
        throw invalid("named struct schema reference is not fully qualified: " + symbol.display());
      }
      return symbol.display();
    }
    throw invalid("struct schema name must be a qualified symbol or (var ...) reference");
  }

  private static Field normalizeStructField(Object argument) {
    ILinearType<?> pair = vector(argument);
    if (pair == null || (pair.count() != 2 && pair.count() != 3)) {
      throw invalid(":struct schema fields must be [name type] or [name properties type]");
    }
    if (pair.count() == 2) {
      return new Field(pair.nth(0), null, normalize(pair.nth(1)));
    }
    Object properties = pair.nth(1);
    if (schemaMap(properties) == null) {
      throw invalid(":struct schema field properties must be a map");
    }
    return new Field(pair.nth(0), properties, normalize(pair.nth(2)));
  }

  private static StructType normalizeStructForms(List<Object> arguments, boolean mutable) {
    if (arguments.isEmpty()) throw invalid(":struct schema requires a qualified name");
    String name = normalizeStructName(arguments.get(0));
    List<Field> fields = new ArrayList<>();
    for (int index = 1; index < arguments.size(); index++) {
      fields.add(normalizeStructField(arguments.get(index)));
    }
    return new StructType(name, mutable, fields);
  }

  /** Parses one source or bytecode field declaration, accepting legacy symbols. */
  public static NamedField normalizeNamedField(Object value) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof Symbol symbol) {
      if (symbol.getNamespace() != null) {
        throw invalid("named value field names must be unqualified symbols");
      }
      return new NamedField(
          symbol.getName(), null, Keyword.create("any"), new Primitive("any"));
    }
    if (raw instanceof String name && !name.isEmpty() && !name.contains("/")) {
      return new NamedField(name, null, Keyword.create("any"), new Primitive("any"));
    }
    if (!(raw instanceof ILinearType<?> pair) || (pair.count() != 2 && pair.count() != 3)) {
      throw invalid("named value fields must be symbols or [name schema] declarations");
    }
    Object rawName = pair.nth(0);
    if (!(rawName instanceof Symbol symbol) || symbol.getNamespace() != null) {
      throw invalid("named value field names must be unqualified symbols");
    }
    Object properties = null;
    Object schema;
    if (pair.count() == 2) {
      schema = pair.nth(1);
    } else {
      properties = pair.nth(1);
      if (schemaMap(properties) == null) {
        throw invalid("named value field properties must be a map");
      }
      schema = pair.nth(2);
    }
    return new NamedField(symbol.getName(), properties, schema, normalize(schema));
  }

  /** Builds the portable schema attached to a named value declaration. */
  public static Object namedTypeSchema(
      String qualifiedName, boolean mutable, NamedField[] fields) {
    ArrayList<Object> values = new ArrayList<>();
    values.add(Keyword.create("struct"));
    if (mutable) {
      values.add(
          hara.lang.data.Map.Standard.from(
              null, Keyword.create("mutable?"), Boolean.TRUE));
    }
    values.add(
        hara.lang.data.List.Standard.from(
            null, Symbol.create("var"), Symbol.create(qualifiedName)));
    for (NamedField field : fields) {
      ArrayList<Object> fieldValues = new ArrayList<>();
      fieldValues.add(Keyword.create(field.name()));
      if (field.properties() != null) fieldValues.add(field.properties());
      fieldValues.add(field.schema());
      values.add(hara.lang.data.Vector.Standard.from(null, fieldValues.toArray()));
    }
    return hara.lang.data.Vector.Standard.from(null, values.toArray());
  }

  private static Type normalizeLonghandMap(
      IMapType<Object, Object> schema, List<Object> children) {
    if (longhandEntry(schema, "fields") == null) return normalizeMap(children);
    List<Field> fields = new ArrayList<>();
    for (Object rawField : longhandValues(schema, "fields", List.of())) {
      IMapType<Object, Object> field = schemaMap(rawField);
      if (field == null) {
        throw invalid("map schema fields must be {:name name :type schema} maps");
      }
      Entry<Object, Object> name = longhandEntry(field, "name");
      Entry<Object, Object> type = longhandEntry(field, "type");
      Object properties = longhandValue(field, "properties");
      if (name == null) throw invalid("map schema field requires :name");
      if (type == null) throw invalid("map schema field requires :type");
      if (properties != null && schemaMap(properties) == null)
        throw invalid("map schema field :properties must be a map");
      fields.add(new Field(name.getValue(), properties, normalize(type.getValue())));
    }
    return new MapType(fields);
  }

  private static Function normalizeLonghandFunction(IMapType<Object, Object> schema) {
    Entry<Object, Object> inputsEntry = longhandEntry(schema, "inputs");
    Entry<Object, Object> outputEntry = longhandEntry(schema, "output");
    if (inputsEntry == null) throw invalid("function schema requires :inputs");
    if (outputEntry == null) throw invalid("function schema requires :output");

    Object inputsValue = inputsEntry.getValue();
    IMapType<Object, Object> inputsMap = schemaMap(inputsValue);
    List<Type> fixed = new ArrayList<>();
    Type rest = null;
    if (inputsMap != null) {
      for (Object value : longhandValues(inputsMap, "fixed", List.of())) {
        fixed.add(normalize(value));
      }
      Entry<Object, Object> restEntry = longhandEntry(inputsMap, "rest");
      if (restEntry != null && HaraBox.unwrap(restEntry.getValue()) != null) {
        rest = normalize(HaraBox.unwrap(restEntry.getValue()));
      }
    } else {
      ILinearType<?> inputs = vector(inputsValue);
      if (inputs == null) {
        throw invalid("function schema :inputs must be a vector or map");
      }
      int index = 0;
      while (index < inputs.count()) {
        if (inputs.nth(index) instanceof Symbol marker
            && marker.getNamespace() == null
            && "&".equals(marker.getName())) {
          if (rest != null || index + 2 != inputs.count()) {
            throw invalid(":fn schema & must precede exactly one rest type");
          }
          rest = normalize(inputs.nth(index + 1));
          index += 2;
        } else {
          fixed.add(normalize(inputs.nth(index++)));
        }
      }
    }
    return new Function(fixed, rest, normalize(outputEntry.getValue()));
  }

  private static FunctionType normalizeLonghandFunctions(List<Object> values) {
    if (values.isEmpty()) {
      throw invalid(":function schema requires at least one :fn schema");
    }
    List<Function> arities = new ArrayList<>();
    for (Object value : values) {
      IMapType<Object, Object> schema = schemaMap(value);
      if (schema != null && longhandEntry(schema, "kind") == null) {
        arities.add(normalizeLonghandFunction(schema));
        continue;
      }
      Type normalized = normalize(value);
      if (!(normalized instanceof FunctionType functions)) {
        throw invalid(":function members must be :fn schemas");
      }
      arities.addAll(functions.arities());
    }
    return new FunctionType(arities);
  }

  private static Type normalizeLonghand(IMapType<Object, Object> schema, String kind) {
    List<Object> children = longhandValues(schema, "children", List.of());
    Type normalized = switch (kind) {
      case "primitive" -> {
        Object name = longhandValue(schema, "name");
        if (name == null && !children.isEmpty()) name = children.get(0);
        if (!(name instanceof Keyword keyword)) {
          throw invalid("primitive schema requires one keyword name");
        }
        String normalizedName = keywordName(keyword);
        yield "integer".equals(normalizedName)
            ? integerType()
            : new Primitive(normalizedName);
      }
      case "reference" -> {
        Object name = longhandValue(schema, "name");
        if (name == null && !children.isEmpty()) name = children.get(0);
        yield normalizeReference(name);
      }
      case "union", "or" -> normalizeUnion(longhandValues(schema, "types", children));
      case "vector" -> {
        Object item = longhandValue(schema, "item");
        if (item == null && !children.isEmpty()) item = children.get(0);
        if (item == null) throw invalid("vector schema requires :item");
        yield new VectorType(normalize(item));
      }
      case "set" -> {
        Object item = longhandValue(schema, "item");
        if (item == null && !children.isEmpty()) item = children.get(0);
        if (item == null) throw invalid("set schema requires :item");
        yield new SetType(normalize(item));
      }
      case "tuple" -> new Tuple(normalizeAll(longhandValues(schema, "items", children)));
      case "map" -> normalizeLonghandMap(schema, children);
      case "struct" -> {
        Object mutableValue = longhandValue(schema, "mutable?");
        boolean mutable = false;
        if (mutableValue != null) {
          if (!(mutableValue instanceof Boolean value)) {
            throw invalid("struct schema :mutable? must be boolean");
          }
          mutable = value;
        }
        Object name = longhandValue(schema, "name");
        if (name == null && !children.isEmpty()) name = children.get(0);
        if (name == null) throw invalid(":struct schema requires a qualified name");
        List<Object> fallback =
            children.isEmpty() ? List.of() : children.subList(1, children.size());
        List<Object> rawFields =
            longhandEntry(schema, "fields") == null
                ? fallback
                : longhandValues(schema, "fields", List.of());
        List<Field> fields = new ArrayList<>();
        for (Object field : rawFields) fields.add(normalizeStructField(field));
        yield new StructType(normalizeStructName(name), mutable, fields);
      }
      case "fn" -> new FunctionType(List.of(normalizeLonghandFunction(schema)));
      case "function" -> normalizeLonghandFunctions(longhandValues(schema, "arities", children));
      case "enum" -> new EnumType(longhandValues(schema, "values", children));
      case "extension" -> {
        Object headValue = longhandValue(schema, "head");
        if (headValue == null) headValue = longhandValue(schema, "name");
        if (!(headValue instanceof Keyword head)) {
          throw invalid("extension schema :head must be a keyword");
        }
        yield new Extension(keywordName(head), longhandValues(schema, "arguments", children));
      }
      case "unknown" -> {
        Object surface = longhandValue(schema, "surface");
        if (surface == null && !children.isEmpty()) surface = children.get(0);
        yield new Unknown(surface == null ? schema : surface);
      }
      default -> throw invalid("unsupported longhand schema kind: " + kind);
    };
    Object properties = longhandValue(schema, "properties");
    if (properties == null) return normalized;
    if (schemaMap(properties) == null) throw invalid("schema :properties must be a map");
    return new Properties(normalized, properties);
  }

  /** Conservative body-derived function facts used by lowering tiers. */
  public static Map<String, Type> inferFunctionTypes(
      String namespace,
      Object[] forms,
      Map<String, Type> declarations,
      Map<String, Type> definitions) {
    Map<String, Type> inferred = new HashMap<>();
    for (Object form : forms) {
      if (!(form instanceof hara.lang.data.List<?> definition) || definition.count() < 3) continue;
      if (!(definition.nth(0) instanceof Symbol operator)
          || operator.getNamespace() != null
          || !("defn".equals(operator.getName()) || "defn-".equals(operator.getName()))) continue;
      if (!(definition.nth(1) instanceof Symbol name)) continue;
      int parametersAt = -1;
      for (int index = 2; index < definition.count(); index++) {
        if (vector(definition.nth(index)) != null) {
          parametersAt = index;
          break;
        }
      }
      String qualified = namespace + "/" + name.getName();
      if (parametersAt < 0) {
        List<Function> arities = new ArrayList<>();
        for (int index = 2; index < definition.count(); index++) {
          if (!(definition.nth(index) instanceof hara.lang.data.List<?> clause)
              || clause.count() < 2
              || vector(clause.nth(0)) == null) continue;
          Object[] single = new Object[Math.toIntExact(clause.count()) + 2];
          single[0] = definition.nth(0);
          single[1] = definition.nth(1);
          for (int item = 0; item < clause.count(); item++) single[item + 2] = clause.nth(item);
          hara.lang.data.List<?> synthetic = hara.lang.data.List.Standard.from(null, single);
          Type type =
              inferFunctionTypes(
                      namespace, new Object[] {synthetic}, declarations, definitions)
                  .get(qualified);
          if (type instanceof FunctionType functions) arities.addAll(functions.arities());
        }
        if (!arities.isEmpty()) inferred.put(qualified, new FunctionType(arities));
        continue;
      }
      ILinearType<?> parameters = vector(definition.nth(parametersAt));
      Function declared = matchingArity(
          resolve(declarations.get(qualified), definitions), parameters);
      Map<String, Type> environment = new HashMap<>();
      List<Type> fixed = new ArrayList<>();
      Type rest = null;
      int fixedIndex = 0;
      boolean variadic = false;
      for (int index = 0; index < parameters.count(); index++) {
        Object parameter = parameters.nth(index);
        if (parameter instanceof Symbol marker
            && marker.getNamespace() == null
            && "&".equals(marker.getName())) {
          variadic = true;
          continue;
        }
        if (!(parameter instanceof Symbol parameterName)) continue;
        Type parameterType = variadic
            ? declared != null && declared.rest() != null ? declared.rest() : unknown()
            : declared != null && fixedIndex < declared.fixed().size()
                ? declared.fixed().get(fixedIndex)
                : unknown();
        environment.put(parameterName.getName(), parameterType);
        if (variadic) rest = parameterType;
        else {
          fixed.add(parameterType);
          fixedIndex++;
        }
      }
      Type output = new Primitive("nil");
      for (int index = parametersAt + 1; index < definition.count(); index++) {
        output = inferExpression(definition.nth(index), environment);
      }
      inferred.put(qualified, new FunctionType(List.of(new Function(fixed, rest, output))));
    }
    return Map.copyOf(inferred);
  }

  private static Type resolve(Type type, Map<String, Type> definitions) {
    HashSet<String> visited = new HashSet<>();
    while (true) {
      if (type instanceof Properties decorated) {
        type = decorated.schema();
        continue;
      }
      if (!(type instanceof Reference reference) || !visited.add(reference.name())) return type;
      Type next = definitions.get(reference.name());
      if (next == null) return type;
      type = next;
    }
  }

  private static Function matchingArity(Type type, ILinearType<?> parameters) {
    if (!(type instanceof FunctionType functions)) return null;
    int fixed = 0;
    boolean variadic = false;
    for (int index = 0; index < parameters.count(); index++) {
      Object parameter = parameters.nth(index);
      if (parameter instanceof Symbol marker && "&".equals(marker.getName())) variadic = true;
      else if (!variadic) fixed++;
    }
    for (Function function : functions.arities()) {
      if (function.fixed().size() == fixed && (function.rest() != null) == variadic) return function;
    }
    return null;
  }

  private static Type inferExpression(Object form, Map<String, Type> environment) {
    if (form == null) return new Primitive("nil");
    if (form instanceof Boolean) return new Primitive("bool");
    if (form instanceof Byte || form instanceof Short || form instanceof Integer || form instanceof Long)
      return new Primitive("long");
    if (form instanceof Float || form instanceof Double) return new Primitive("float");
    if (form instanceof java.math.BigInteger integer)
      return NumUtils.isLongValue(integer) ? new Primitive("long") : new Primitive("bigint");
    if (form instanceof hara.lang.data.HaraCharacter || form instanceof Character)
      return new Primitive("char");
    if (form instanceof java.util.regex.Pattern) return new Primitive("regex");
    if (form instanceof String) return new Primitive("str");
    if (form instanceof Keyword) return new Primitive("keyword");
    if (form instanceof Symbol symbol)
      return environment.getOrDefault(symbol.getName(), unknown());
    ILinearType<?> vector = vector(form);
    if (vector != null) {
      List<Type> members = new ArrayList<>();
      for (int index = 0; index < vector.count(); index++)
        pushJoined(members, inferExpression(vector.nth(index), environment));
      return new VectorType(join(members));
    }
    if (form instanceof hara.lang.protocol.IMapType<?, ?> map) {
      List<Field> fields = new ArrayList<>();
      for (Object item : map) {
        Entry<?, ?> entry = (Entry<?, ?>) item;
        fields.add(new Field(entry.getKey(), null, inferExpression(entry.getValue(), environment)));
      }
      return new MapType(fields);
    }
    if (form instanceof hara.lang.protocol.ISetType<?> set) {
      List<Type> members = new ArrayList<>();
      for (Object value : set) pushJoined(members, inferExpression(value, environment));
      return new SetType(join(members));
    }
    if (!(form instanceof hara.lang.data.List<?> list) || list.count() == 0
        || !(list.nth(0) instanceof Symbol operator)) return unknown();
    return inferList(list, operator.getName(), environment);
  }

  private static Type inferList(
      hara.lang.data.List<?> list, String operator, Map<String, Type> environment) {
    switch (operator) {
      case "do": {
        Type output = new Primitive("nil");
        for (int index = 1; index < list.count(); index++)
          output = inferExpression(list.nth(index), environment);
        return output;
      }
      case "if": {
        List<Type> branches = new ArrayList<>();
        for (int index = 2; index < list.count(); index++)
          pushJoined(branches, inferExpression(list.nth(index), environment));
        return join(branches);
      }
      case "let": {
        Map<String, Type> nested = new HashMap<>(environment);
        ILinearType<?> bindings = list.count() > 1 ? vector(list.nth(1)) : null;
        if (bindings != null) {
          for (int index = 0; index + 1 < bindings.count(); index += 2) {
            if (bindings.nth(index) instanceof Symbol name)
              nested.put(name.getName(), inferExpression(bindings.nth(index + 1), nested));
          }
        }
        Type output = new Primitive("nil");
        for (int index = 2; index < list.count(); index++)
          output = inferExpression(list.nth(index), nested);
        return output;
      }
      case "+", "-", "*", "mod": {
        List<Type> operands = new ArrayList<>();
        for (int index = 1; index < list.count(); index++)
          pushJoined(operands, inferExpression(list.nth(index), environment));
        Type joined = join(operands);
        if (joined instanceof Primitive primitive
            && List.of("int", "long", "bigint", "float").contains(primitive.name())) return joined;
        if (joined instanceof Union union && union.types().stream().allMatch(HalcSchema::isLongAlias))
          return new Primitive("long");
        return new Primitive("number");
      }
      case "/": return new Primitive("number");
      case "=", "<", "<=", ">", ">=", "instance?": return new Primitive("bool");
      case "count": return new Primitive("long");
      default: return unknown();
    }
  }

  private static Unknown unknown() {
    return new Unknown(Symbol.create("?"));
  }

  private static Type integerType() {
    return new Union(List.of(new Primitive("long"), new Primitive("bigint")));
  }

  private static boolean isLongAlias(Type type) {
    return type instanceof Primitive primitive
        && ("int".equals(primitive.name()) || "long".equals(primitive.name()));
  }

  private static Type join(List<Type> members) {
    if (members.isEmpty()) return unknown();
    return members.size() == 1 ? members.get(0) : new Union(members);
  }

  private static void pushJoined(List<Type> output, Type type) {
    if (type instanceof Union union) {
      for (Type member : union.types()) pushUnique(output, member);
    } else pushUnique(output, type);
  }

  /** Canonical reader-form bridge used by the cross-runtime HBC schema codec. */
  public static Object readSurface(String source) {
    Object[] forms = HaraLanguage.readAll(source, "hbc:schema");
    if (forms.length != 1) throw invalid("schema surface must contain one form");
    return forms[0];
  }

  /** Canonical readable spelling used by the cross-runtime HBC schema codec. */
  public static String displaySurface(Object value) {
    return G.display(value);
  }

  private static Type normalizeUnion(List<Object> arguments) {
    if (arguments.isEmpty()) throw invalid(":or schema requires at least one member");
    List<Type> members = new ArrayList<>();
    for (Object argument : arguments) {
      Type normalized = normalize(argument);
      if (normalized instanceof Union union) {
        for (Type member : union.types()) pushUnique(members, member);
      } else {
        pushUnique(members, normalized);
      }
    }
    return members.size() == 1 ? members.get(0) : new Union(members);
  }

  private static Type normalizeMap(List<Object> arguments) {
    List<Field> fields = new ArrayList<>();
    for (Object argument : arguments) {
      ILinearType<?> pair = vector(argument);
      if (pair == null || (pair.count() != 2 && pair.count() != 3)) {
        throw invalid(":map schema fields must be [name type] or [name properties type]");
      }
      if (pair.count() == 2) {
        fields.add(new Field(pair.nth(0), null, normalize(pair.nth(1))));
      } else {
        Object properties = pair.nth(1);
        if (schemaMap(properties) == null)
          throw invalid(":map schema field properties must be a map");
        fields.add(new Field(pair.nth(0), properties, normalize(pair.nth(2))));
      }
    }
    return new MapType(fields);
  }

  private static Function normalizeFunction(ILinearType<?> function) {
    if (function.count() != 3
        || !(function.nth(0) instanceof Keyword head)
        || !"fn".equals(head.getName())) {
      throw invalid(":fn schema must be [:fn [inputs ...] output]");
    }
    ILinearType<?> inputs = vector(function.nth(1));
    if (inputs == null) {
      throw invalid(":fn schema inputs must be a vector");
    }
    List<Type> fixed = new ArrayList<>();
    Type rest = null;
    int index = 0;
    while (index < inputs.count()) {
      if (inputs.nth(index) instanceof Symbol marker
          && marker.getNamespace() == null
          && "&".equals(marker.getName())) {
        if (rest != null || index + 2 != inputs.count()) {
          throw invalid(":fn schema & must precede exactly one rest type");
        }
        rest = normalize(inputs.nth(index + 1));
        index += 2;
      } else {
        fixed.add(normalize(inputs.nth(index++)));
      }
    }
    return new Function(fixed, rest, normalize(function.nth(2)));
  }

  private static List<Type> normalizeAll(List<Object> values) {
    List<Type> output = new ArrayList<>(values.size());
    for (Object value : values) output.add(normalize(value));
    return output;
  }

  private static List<Object> values(ILinearType<?> input, int start) {
    List<Object> output = new ArrayList<>(Math.toIntExact(input.count()) - start);
    for (int index = start; index < input.count(); index++) output.add(input.nth(index));
    return output;
  }

  private static ILinearType<?> vector(Object value) {
    return value instanceof ILinearType<?> linear && "[".equals(linear.startString())
        ? linear
        : null;
  }

  private static void requireCount(String head, List<Object> arguments, int expected) {
    if (arguments.size() != expected) {
      throw invalid(
          ":"
              + head
              + " schema expects "
              + expected
              + (expected == 1 ? " argument, got " : " arguments, got ")
              + arguments.size());
    }
  }

  private static void pushUnique(List<Type> output, Type value) {
    if (!output.contains(value)) output.add(value);
  }

  private static HaraException invalid(String detail) {
    return new HaraException(detail);
  }
}
