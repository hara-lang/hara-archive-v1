package hara.truffle;

import hara.kernel.base.RT;
import hara.kernel.base.Namespace;
import hara.kernel.base.Var;
import hara.lang.base.G;
import hara.lang.data.Keyword;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import java.math.BigInteger;
import java.util.ArrayList;
import java.util.Iterator;
import java.util.List;
import java.util.Objects;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.PolyglotException;
import org.graalvm.polyglot.Value;

final class HaraDifferentialRunner {
  private static final Keyword ID = key("id");
  private static final Keyword SOURCE = key("source");
  private static final Keyword CLASSIFICATION = key("classification");
  private static final Keyword CAPABILITY = key("capability");
  private static final Keyword REASON = key("reason");
  private static final Keyword ISSUE = key("issue");
  private static final Keyword CASES = key("cases");
  private static final Keyword PORTABLE = key("portable");
  private static final Keyword CAPABILITY_SPECIFIC = key("capability-specific");
  private static final Keyword KNOWN_GAP = key("known-gap");

  static final class TestCase {
    private final Keyword id;
    private final String source;
    private final Keyword classification;
    private final Keyword capability;
    private final String reason;
    private final String issue;

    TestCase(
        Keyword id,
        String source,
        Keyword classification,
        Keyword capability,
        String reason,
        String issue) {
      this.id = id;
      this.source = source;
      this.classification = classification;
      this.capability = capability;
      this.reason = reason;
      this.issue = issue;
    }

    Keyword id() {
      return id;
    }

    String source() {
      return source;
    }

    Keyword classification() {
      return classification;
    }

    Keyword capability() {
      return capability;
    }

    String reason() {
      return reason;
    }

    String issue() {
      return issue;
    }

    boolean portable() {
      return PORTABLE.equals(classification);
    }

    boolean capabilitySpecific() {
      return CAPABILITY_SPECIFIC.equals(classification);
    }

    boolean knownGap() {
      return KNOWN_GAP.equals(classification);
    }
  }

  static final class Outcome {
    private final boolean success;
    private final String kind;
    private final String value;

    private Outcome(boolean success, String kind, String value) {
      this.success = success;
      this.kind = kind;
      this.value = value;
    }

    static Outcome value(String kind, String value) {
      return new Outcome(true, kind, value);
    }

    static Outcome error(String kind) {
      return new Outcome(false, kind, null);
    }

    @Override
    public boolean equals(Object other) {
      if (this == other) return true;
      if (!(other instanceof Outcome)) return false;
      Outcome that = (Outcome) other;
      return success == that.success
          && Objects.equals(kind, that.kind)
          && Objects.equals(value, that.value);
    }

    @Override
    public int hashCode() {
      return Objects.hash(success, kind, value);
    }

    @Override
    public String toString() {
      return success ? kind + "(" + value + ")" : "error(" + kind + ")";
    }
  }

  static final class Result {
    private final TestCase testCase;
    private final Outcome jvm;
    private final Outcome truffle;

    Result(TestCase testCase, Outcome jvm, Outcome truffle) {
      this.testCase = testCase;
      this.jvm = jvm;
      this.truffle = truffle;
    }

    Outcome jvm() {
      return jvm;
    }

    Outcome truffle() {
      return truffle;
    }

    boolean excluded() {
      return testCase.capabilitySpecific();
    }

    boolean matches() {
      return !excluded() && jvm.equals(truffle);
    }
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  static List<TestCase> readCases(Object manifest) {
    IMapType root = requireMap(manifest, "parity manifest");
    Object rawCases = root.lookup(CASES);
    if (!(rawCases instanceof ILinearType)) {
      throw new IllegalArgumentException("Parity manifest must contain a :cases collection");
    }
    List<TestCase> cases = new ArrayList<>();
    for (Object rawCase : (ILinearType<?>) rawCases) {
      IMapType item = requireMap(rawCase, "parity case");
      Keyword id = requireKeyword(item.lookup(ID), ":id");
      String source = requireString(item.lookup(SOURCE), id + " :source");
      Keyword classification = requireKeyword(item.lookup(CLASSIFICATION), id + " :classification");
      if (PORTABLE.equals(classification)) {
        cases.add(new TestCase(id, source, classification, null, null, null));
      } else if (CAPABILITY_SPECIFIC.equals(classification)) {
        Keyword capability = requireKeyword(item.lookup(CAPABILITY), id + " :capability");
        String reason = requireString(item.lookup(REASON), id + " :reason");
        cases.add(new TestCase(id, source, classification, capability, reason, null));
      } else if (KNOWN_GAP.equals(classification)) {
        String issue = requireString(item.lookup(ISSUE), id + " :issue");
        String reason = requireString(item.lookup(REASON), id + " :reason");
        cases.add(new TestCase(id, source, classification, null, reason, issue));
      } else {
        throw new IllegalArgumentException(
            id + " has unsupported classification " + classification);
      }
    }
    return cases;
  }

  static Result run(TestCase testCase) {
    if (testCase.capabilitySpecific()) {
      return new Result(testCase, null, null);
    }
    return new Result(testCase, evaluateJvm(testCase.source()), evaluateTruffle(testCase.source()));
  }

  private static Outcome evaluateJvm(String source) {
    RT.Instance<Object> runtime = new RT.Instance<>(null, "differential-jvm");
    try {
      installIteratorNamespace(runtime);
      return normalizeJvm(runtime.eval(runtime.readString(source)));
    } catch (Throwable error) {
      return Outcome.error(normalizeError(error));
    }
  }

  private static void installIteratorNamespace(RT.Instance<Object> runtime) {
    Namespace iterator = runtime.addNamespace(hara.lang.data.Symbol.create("std.native.Iter"));
    for (String name : List.of("iter-next?", "iter-next", "iter-close")) {
      Object function = runtime.eval(hara.lang.data.Symbol.create(name));
      iterator.mappings.put(hara.lang.data.Symbol.create(name), new Var("std.native.Iter", function));
    }
    runtime
        .getCurrentNs()
        .aliases
        .put(hara.lang.data.Symbol.create("Iter"), iterator);
  }

  private static Outcome evaluateTruffle(String source) {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      try {
        return normalizeTruffle(context.eval(HaraLanguage.ID, source));
      } catch (PolyglotException error) {
        return Outcome.error(normalizeError(error));
      }
    }
  }

  private static Outcome normalizeJvm(Object value) {
    if (value == null) return Outcome.value("nil", "nil");
    if (value instanceof Boolean) return Outcome.value("boolean", value.toString());
    if (value instanceof String) return Outcome.value("string", value.toString());
    if (value instanceof Character)
      return Outcome.value("character", Integer.toString((Character) value));
    if (isIntegral(value)) return Outcome.value("integer", value.toString());
    if (value instanceof Float || value instanceof Double) {
      return Outcome.value("floating", floatingValue(((Number) value).doubleValue()));
    }
    if (value instanceof Iterable) {
      return normalizeSequence(((Iterable<?>) value).iterator());
    }
    return Outcome.value("display", G.display(value));
  }

  private static Outcome normalizeTruffle(Value value) {
    if (value.isNull()) return Outcome.value("nil", "nil");
    if (value.isBoolean()) return Outcome.value("boolean", Boolean.toString(value.asBoolean()));
    if (value.isString()) return Outcome.value("string", value.asString());
    if (value.fitsInLong()) return Outcome.value("integer", Long.toString(value.asLong()));
    if (value.fitsInBigInteger()) {
      return Outcome.value("integer", value.as(BigInteger.class).toString());
    }
    if (value.fitsInDouble()) {
      return Outcome.value("floating", floatingValue(value.asDouble()));
    }
    if (value.hasArrayElements()) {
      List<String> elements = new ArrayList<>();
      for (long i = 0; i < value.getArraySize(); i++) {
        elements.add(normalizeTruffle(value.getArrayElement(i)).toString());
      }
      return Outcome.value("sequence", elements.toString());
    }
    return Outcome.value("display", value.toString());
  }

  private static Outcome normalizeSequence(Iterator<?> values) {
    List<String> elements = new ArrayList<>();
    while (values.hasNext()) {
      elements.add(normalizeJvm(values.next()).toString());
    }
    return Outcome.value("sequence", elements.toString());
  }

  private static boolean isIntegral(Object value) {
    return value instanceof Byte
        || value instanceof Short
        || value instanceof Integer
        || value instanceof Long
        || value instanceof BigInteger;
  }

  private static String floatingValue(double value) {
    if (Double.isNaN(value)) return "NaN";
    if (value == Double.POSITIVE_INFINITY) return "Infinity";
    if (value == Double.NEGATIVE_INFINITY) return "-Infinity";
    return Double.toHexString(value);
  }

  private static String normalizeError(Throwable error) {
    while (error.getCause() != null && error.getCause() != error) {
      error = error.getCause();
    }
    String message = error.getMessage() == null ? "" : error.getMessage().toLowerCase();
    if ((message.contains("expect") && message.contains("argument"))
        || message.contains("arity")
        || message.contains("wrong number of args")
        || message.matches(".*expects? [0-9]+ arguments?.*")) {
      return "arity";
    }
    if (message.contains("unbound symbol") || message.contains("cannot find symbol")) {
      return "unbound-symbol";
    }
    if (message.contains("divide by zero") || message.contains("division by zero")) {
      return "divide-by-zero";
    }
    return error.getClass().getSimpleName();
  }

  @SuppressWarnings("rawtypes")
  private static IMapType requireMap(Object value, String label) {
    if (value instanceof IMapType) return (IMapType) value;
    throw new IllegalArgumentException(label + " must be a map");
  }

  private static Keyword requireKeyword(Object value, String label) {
    if (value instanceof Keyword) return (Keyword) value;
    throw new IllegalArgumentException(label + " must be a keyword");
  }

  private static String requireString(Object value, String label) {
    if (value instanceof String && !((String) value).isBlank()) return (String) value;
    throw new IllegalArgumentException(label + " must be a non-empty string");
  }

  private static Keyword key(String name) {
    return Keyword.create(null, name);
  }
}
