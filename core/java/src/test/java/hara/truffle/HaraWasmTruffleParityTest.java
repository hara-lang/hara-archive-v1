package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertTrue;
import static org.junit.Assume.assumeTrue;
import static org.junit.Assert.fail;

import hara.kernel.base.Parser;
import hara.lang.data.Keyword;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.spec.SpecRegistry;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashSet;
import java.util.Locale;
import java.util.Set;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.PolyglotException;
import org.graalvm.polyglot.Value;
import org.junit.Test;

/** Executes the shared Rust-WASM/Truffle parity corpus. */
@org.junit.experimental.categories.Category(hara.spec.RegistryConformance.class)
public class HaraWasmTruffleParityTest {
  private static final Path CORPUS =
      SpecRegistry.resolve(
          "01-lang/001-language/draft/conformance/parity/wasm-truffle.edn");
  private static final Path ARTIFACT =
      Path.of("rust/raw/target/wasm32-unknown-unknown/browser-release/hara-wasm-vm.wasm");

  private static Keyword key(String name) {
    return Keyword.create(null, name);
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  @Test
  public void sharedCorpusMatchesRustWasmAndTruffle() throws Exception {
    assumeTrue("build rust/raw before parity tests: " + ARTIFACT, Files.isRegularFile(ARTIFACT));
    IMapType manifest = readManifest();
    ILinearType<?> cases = requireCases(manifest);
    assertEquals(1024, cases.count());

    Path root = Files.createTempDirectory("hara-wasm-truffle-parity-");
    Path extension = root.resolve("test/rust/parity");
    Files.createDirectories(extension);
    HaraExtensionTestProject.write(
        extension,
        "{:namespace \"test.rust.parity\" :version \"0.2.0\" :provider :wasm "
            + ":module \"hara.wasm\" :abi :core.v1 "
            + ":exports {\"eval_i64\" {:args [:utf8] :returns :i64} "
            + "\"eval_error_code\" {:args [:utf8] :returns :i32}} :capabilities []}");
    Files.copy(ARTIFACT, extension.resolve("hara.wasm"));
    String previous = System.getProperty("hara.extensions.path");
    System.setProperty("hara.extensions.path", root.toString());
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(HaraLanguage.ID, "(ns app (:require [test.rust.parity :as rust]))");
      Set<String> ids = new HashSet<>();
      for (Object item : cases) {
        IMapType testCase = (IMapType) item;
        String id = keywordText(requireKeyword(testCase, "id"));
        assertTrue("duplicate parity case " + id, ids.add(id));
        String source = requireString(testCase, "source");
        String setup = optionalString(testCase, "setup");
        String program = setup == null ? source : setup + "\n" + source;
        IMapType expect = requireMap(testCase, "expect");
        Object expectedValue = expect.lookup(key("value"));
        Object expectedError = expect.lookup(key("error"));
        assertFalse(
            id + " must specify exactly one expectation",
            (expectedValue == null) == (expectedError == null));
        if (expectedValue != null) {
          long expected = ((Number) expectedValue).longValue();
          Value truffle = evaluateTruffle(context, setup, source, id);
          Value wasm = context.eval(HaraLanguage.ID, "(rust/eval_i64 " + quote(program) + ")");
          assertTrue(id + " Truffle result must be numeric", truffle.fitsInLong());
          assertTrue(id + " Rust WASM result must be numeric", wasm.fitsInLong());
          assertEquals(id + " expected Truffle result", expected, truffle.asLong());
          assertEquals(id + " expected Rust WASM result", expected, wasm.asLong());
          assertEquals(id + " Rust WASM/Truffle mismatch", truffle.asLong(), wasm.asLong());
        } else {
          String expectedCode = keywordText((Keyword) expectedError);
          String actualCode = null;
          try {
            evaluateTruffle(context, setup, source, id);
            fail(id + " should fail with " + expectedCode);
          } catch (PolyglotException error) {
            actualCode = normalizeError(error);
          }
          assertEquals(id + " Truffle error", expectedCode, actualCode);
          Value wasmCode =
              context.eval(HaraLanguage.ID, "(rust/eval_error_code " + quote(program) + ")");
          assertEquals(id + " Rust WASM error", errorNumber(expectedCode), wasmCode.asLong());
        }
      }
      assertEquals("all parity IDs must be unique", cases.count(), ids.size());
    } finally {
      if (previous == null) System.clearProperty("hara.extensions.path");
      else System.setProperty("hara.extensions.path", previous);
      Files.deleteIfExists(extension.resolve("hara.wasm"));
      Files.deleteIfExists(extension.resolve("project.edn"));
      Files.deleteIfExists(extension);
      Files.deleteIfExists(extension.getParent());
      Files.deleteIfExists(extension.getParent().getParent());
      Files.deleteIfExists(root);
    }
  }

  @SuppressWarnings("rawtypes")
  @Test
  public void corpusHasRequiredMetadataAndOrigins() throws Exception {
    IMapType manifest = readManifest();
    ILinearType<?> cases = requireCases(manifest);
    assertEquals(1024, cases.count());
    assertEquals(256L, ((Number) manifest.lookup(key("curated"))).longValue());
    assertEquals(768L, ((Number) manifest.lookup(key("generated"))).longValue());
    assertNotNull(manifest.lookup(key("seed")));
    int curated = 0;
    int generated = 0;
    for (Object item : cases) {
      IMapType testCase = (IMapType) item;
      String origin = keywordText(requireKeyword(testCase, "origin"));
      if ("curated".equals(origin)) curated++;
      else if ("generated".equals(origin)) generated++;
      else fail("unknown parity origin " + origin);
      assertNotNull(testCase.lookup(key("group")));
    }
    assertEquals(256, curated);
    assertEquals(768, generated);
  }

  @SuppressWarnings("rawtypes")
  private static IMapType readManifest() throws Exception {
    return (IMapType) Parser.LispReader.readString(Files.readString(CORPUS), null);
  }

  @SuppressWarnings("rawtypes")
  private static ILinearType<?> requireCases(IMapType manifest) {
    Object value = manifest.lookup(key("cases"));
    assertTrue("parity manifest must contain :cases", value instanceof ILinearType);
    return (ILinearType<?>) value;
  }

  @SuppressWarnings("rawtypes")
  private static IMapType requireMap(IMapType map, String name) {
    Object value = map.lookup(key(name));
    assertTrue(name + " must be a map", value instanceof IMapType);
    return (IMapType) value;
  }

  @SuppressWarnings("rawtypes")
  private static Keyword requireKeyword(IMapType map, String name) {
    Object value = map.lookup(key(name));
    assertTrue(name + " must be a keyword", value instanceof Keyword);
    return (Keyword) value;
  }

  @SuppressWarnings("rawtypes")
  private static String requireString(IMapType map, String name) {
    Object value = map.lookup(key(name));
    assertTrue(name + " must be a string", value instanceof String);
    return (String) value;
  }

  @SuppressWarnings("rawtypes")
  private static String optionalString(IMapType map, String name) {
    Object value = map.lookup(key(name));
    if (value == null) return null;
    assertTrue(name + " must be a string", value instanceof String);
    return (String) value;
  }

  private static Value evaluateTruffle(Context context, String setup, String source, String id) {
    if (setup != null) context.eval(HaraLanguage.ID, setup);
    try {
      return context.eval(HaraLanguage.ID, source);
    } catch (PolyglotException error) {
      throw error;
    } catch (RuntimeException error) {
      throw new HaraException("parity/truffle-evaluation-failed " + id + ": " + error.getMessage());
    }
  }

  private static String normalizeError(Throwable error) {
    String message = error.getMessage() == null ? "" : error.getMessage().toLowerCase(Locale.ROOT);
    if (message.contains("division by zero") || message.contains("by zero"))
      return "arithmetic/divide-by-zero";
    if (message.contains("unbound symbol") || message.contains("unbound var"))
      return "unbound-symbol";
    if (message.contains("recur must") || message.contains("unsupported")) return "unsupported";
    if (message.contains("index") || message.contains("out of bounds")) return "index";
    if (message.contains("arity")
        || message.contains("expects 1 argument")
        || message.contains("expects at least")
        || message.contains("expected ") && message.contains("arguments, received")) return "arity";
    if (message.contains("parse")
        || message.contains("unexpected")
        || message.contains("unclosed")
        || message.contains("unmatched delimiter")) return "parse";
    return "type";
  }

  private static long errorNumber(String code) {
    switch (code) {
      case "parse":
        return 1;
      case "unbound-symbol":
        return 2;
      case "arity":
        return 3;
      case "type":
        return 4;
      case "arithmetic/divide-by-zero":
        return 5;
      case "index":
        return 6;
      case "unsupported":
        return 7;
      default:
        throw new IllegalArgumentException("unknown parity error code " + code);
    }
  }

  private static String keywordText(Keyword keyword) {
    return keyword.getNamespace() == null
        ? keyword.getName()
        : keyword.getNamespace() + "/" + keyword.getName();
  }

  private static String quote(String source) {
    return "\"" + source.replace("\\", "\\\\").replace("\"", "\\\"") + "\"";
  }
}
