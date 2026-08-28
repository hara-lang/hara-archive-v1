package hara.truffle.bytecode;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.truffle.HtaValueCodec;
import hara.truffle.HalcSchema;
import hara.truffle.HaraLanguage;
import hara.lang.base.G;
import hara.truffle.bytecode.HbcProgram.Function;
import hara.truffle.bytecode.HbcProgram.Instruction;
import hara.truffle.bytecode.HbcProgram.Opcode;
import hara.truffle.bytecode.HbcProgram.Primitive;
import java.util.Arrays;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import java.math.BigInteger;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.Source;
import org.graalvm.polyglot.io.ByteSequence;
import org.junit.Test;

public class HbcCodecTest {
  @Test
  public void alphaTypedProgramsRoundTripCanonically() {
    HbcProgram base = arithmeticProgram();
    HbcProgram program =
        new HbcProgram(
            "demo",
            base.constants(),
            base.varMetadata(),
            Map.of(
                "demo/Customer",
                new HalcSchema.MapType(
                    List.of(
                        new HalcSchema.Field(
                            hara.lang.data.Keyword.create("id"),
                            null,
                            new HalcSchema.Primitive("int")))),
                "demo/Labels",
                new HalcSchema.SetType(new HalcSchema.Primitive("keyword")),
                "demo/Handle",
                new HalcSchema.Properties(
                    new HalcSchema.Primitive("str"),
                    HalcSchema.readSurface("{:title \"Display handle\" :version 2 :owner :accounts :min-count 1 :max-count 32}")),
                "demo/Profile",
                new HalcSchema.Properties(
                    new HalcSchema.MapType(
                        List.of(
                            new HalcSchema.Field(
                                hara.lang.data.Keyword.create("nickname"),
                                HalcSchema.readSurface("{:required true :description \"Display nickname\" :default \"Anonymous\"}"),
                                new HalcSchema.Primitive("str")))),
                    HalcSchema.readSurface("{:title \"User profile\" :version 2 :owner :accounts :closed true}"))),
            Map.of(
                "demo/add",
                new HalcSchema.FunctionType(
                    List.of(
                        new HalcSchema.Function(
                            List.of(
                                new HalcSchema.Primitive("int"),
                                new HalcSchema.Primitive("int")),
                            null,
                            new HalcSchema.Primitive("int"))))),
            Map.of(
                "demo/inferred",
                new HalcSchema.FunctionType(
                    List.of(
                        new HalcSchema.Function(
                            List.of(), null, new HalcSchema.Primitive("int"))))),
            base.functions(),
            base.entry());
    byte[] first = HbcCodec.encode(program);
    assertArrayEquals(new byte[] {'H', 'B', 'C', '0'}, Arrays.copyOf(first, 4));
    HbcProgram decoded = HbcCodec.decode(first);
    assertArrayEquals(first, HbcCodec.encode(decoded));

    assertTrue(decoded.schemaTypes().get("demo/Labels") instanceof HalcSchema.SetType);
    assertTrue(decoded.schemaTypes().get("demo/Profile") instanceof HalcSchema.Properties);
    HalcSchema.Properties profile =
        (HalcSchema.Properties) decoded.schemaTypes().get("demo/Profile");
    assertTrue(profile.properties() != null);
    assertTrue(profile.schema() instanceof HalcSchema.MapType);
    HalcSchema.MapType profileMap = (HalcSchema.MapType) profile.schema();
    assertEquals(1, profileMap.fields().size());
    assertTrue(profileMap.fields().get(0).properties() != null);
  }

  @Test
  public void corruptionIsRejectedBeforePayloadDecode() {
    byte[] artifact = HbcCodec.encode(arithmeticProgram());
    artifact[12] ^= 1;
    HbcFormatException failure = assertThrows(HbcFormatException.class, () -> HbcCodec.decode(artifact));
    assertEquals("bytecode artifact checksum mismatch", failure.getMessage());
  }

  @Test
  public void canonicalHtaSupportsFloatingConstants() {
    byte[] encoded = HtaValueCodec.encode(1.5d);
    assertEquals(1.5d, (Double) HtaValueCodec.decodeCanonical(encoded), 0.0d);
  }

  @Test
  public void rejectsNonFiniteMetadata() {
    HbcProgram.MetadataValue nonFinite =
        new HbcProgram.MetadataValue(HbcProgram.MetadataValue.Kind.FLOAT, Double.NaN);
    HbcProgram base = arithmeticProgram();
    HbcProgram program =
        new HbcProgram(
            base.constants(),
            List.of(List.of(new HbcProgram.MetadataEntry(nonFinite, nonFinite))),
            base.functions(),
            base.entry());
    assertThrows(HbcFormatException.class, () -> HbcCodec.encode(program));
  }

  @Test
  public void canonicalizesMetadataIntegerWidths() {
    HbcProgram base = arithmeticProgram();
    HbcProgram program =
        new HbcProgram(
            base.constants(),
            List.of(
                List.of(
                    new HbcProgram.MetadataEntry(
                        new HbcProgram.MetadataValue(
                            HbcProgram.MetadataValue.Kind.KEYWORD,
                            hara.lang.data.Keyword.create("small")),
                        new HbcProgram.MetadataValue(
                            HbcProgram.MetadataValue.Kind.BIG_INTEGER, BigInteger.valueOf(42))),
                    new HbcProgram.MetadataEntry(
                        new HbcProgram.MetadataValue(
                            HbcProgram.MetadataValue.Kind.KEYWORD,
                            hara.lang.data.Keyword.create("large")),
                        new HbcProgram.MetadataValue(
                            HbcProgram.MetadataValue.Kind.BIG_INTEGER,
                            BigInteger.ONE.shiftLeft(63))))),
            base.functions(),
            base.entry());

    HbcProgram decoded = HbcCodec.decode(HbcCodec.encode(program));
    assertEquals(
        HbcProgram.MetadataValue.Kind.NUMBER,
        decoded.varMetadata().get(0).get(0).value().kind());
    assertEquals(42L, decoded.varMetadata().get(0).get(0).value().value());
    assertEquals(
        HbcProgram.MetadataValue.Kind.BIG_INTEGER,
        decoded.varMetadata().get(0).get(1).value().kind());
    assertEquals(
        BigInteger.ONE.shiftLeft(63), decoded.varMetadata().get(0).get(1).value().value());
    assertArrayEquals(HbcCodec.encode(program), HbcCodec.encode(decoded));
  }

  @Test
  public void invalidStackProgramsNeverReachExecution() {
    Function invalid =
        new Function(
            null,
            false,
            0,
            false,
            0,
            0,
            0,
            List.of(Instruction.of(Opcode.RETURN)),
            Arrays.asList((HbcProgram.Position) null),
            List.of());
    HbcFormatException failure =
        assertThrows(
            HbcFormatException.class,
            () -> HbcValidator.validate(new HbcProgram(List.of(), List.of(), List.of(invalid), 0)));
    assertTrue(failure.getMessage().contains("return with stack height 0"));
  }

  @Test
  public void polyglotExecutesEncodedHbc3() throws Exception {
    Source source =
        Source.newBuilder(
                HaraLanguage.ID, ByteSequence.create(HbcCodec.encode(arithmeticProgram())), "sum.hbc")
            .mimeType(HaraLanguage.BYTECODE_MIME_TYPE)
            .build();
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(42L, context.eval(source).asLong());
    }
  }

  @Test
  public void closuresAndCallsExecuteInsideThePortableMachine() throws Exception {
    Function addCaptured =
        new Function(
            "add-captured",
            false,
            1,
            false,
            1,
            2,
            2,
            List.of(
                new Instruction(Opcode.LOAD_LOCAL, 0, 0, 0),
                new Instruction(Opcode.LOAD_LOCAL, 1, 0, 0),
                new Instruction(Opcode.PRIMITIVE, Primitive.ADD.id(), 2, 0),
                Instruction.of(Opcode.RETURN)),
            Arrays.asList(null, null, null, null),
            List.of());
    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            0,
            2,
            List.of(
                new Instruction(Opcode.CONSTANT, 0, 0, 0),
                new Instruction(Opcode.CLOSURE, 1, 1, 0),
                new Instruction(Opcode.CONSTANT, 1, 0, 0),
                new Instruction(Opcode.CALL, 1, 0, 0),
                Instruction.of(Opcode.RETURN)),
            Arrays.asList(null, null, null, null, null),
            List.of());
    HbcProgram program = new HbcProgram(List.of(19L, 23L), List.of(), List.of(entry, addCaptured), 0);
    Source source =
        Source.newBuilder(HaraLanguage.ID, ByteSequence.create(HbcCodec.encode(program)), "closure.hbc")
            .mimeType(HaraLanguage.BYTECODE_MIME_TYPE)
            .build();
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(42L, context.eval(source).asLong());
    }
  }

  @Test
  public void portablePrimitivesCannotBeRedirectedByCallerMacros() throws Exception {
    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            0,
            1,
            List.of(
                new Instruction(Opcode.CONSTANT, 0, 0, 0),
                new Instruction(Opcode.PRIMITIVE, Primitive.COUNT.id(), 1, 0),
                Instruction.of(Opcode.RETURN)),
            Arrays.asList(null, null, null),
            List.of());
    HbcProgram program =
        new HbcProgram(
            List.of(hara.lang.data.Vector.Standard.from(null, 1L, 2L, 3L)),
            List.of(),
            List.of(entry),
            0);
    Source source =
        Source.newBuilder(
                HaraLanguage.ID,
                ByteSequence.create(HbcCodec.encode(program)),
                "primitive-shadow.hbc")
            .mimeType(HaraLanguage.BYTECODE_MIME_TYPE)
            .build();
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(
          HaraLanguage.ID,
          "(do (ns primitive.shadow) (defmacro count [& values] nil))");
      assertEquals(3L, context.eval(source).asLong());
    }
  }

  @Test
  public void executesRegistryAndIntrinsicOpcodeTriplet() throws Exception {
    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            0,
            5,
            List.of(
                new Instruction(Opcode.INTRINSIC_VALUE, 0, 0, 0),
                new Instruction(Opcode.CONSTANT, 1, 0, 0),
                new Instruction(Opcode.CONSTANT, 2, 0, 0),
                new Instruction(Opcode.CALL, 2, 0, 0),
                new Instruction(Opcode.CONSTANT, 3, 0, 0),
                new Instruction(Opcode.PROTOCOL_CALL, 4, 1, 0),
                new Instruction(Opcode.CONSTANT, 1, 0, 0),
                new Instruction(Opcode.INTRINSIC_CALL, 5, 1, 0),
                new Instruction(Opcode.CONSTANT, 1, 0, 0),
                new Instruction(Opcode.CONSTANT, 2, 0, 0),
                new Instruction(Opcode.INTRINSIC_CALL, 0, 2, 0),
                new Instruction(Opcode.BUILD_VECTOR, 4, 0, 0),
                Instruction.of(Opcode.RETURN)),
            Arrays.asList(
                null,
                null,
                null,
                null,
                null,
                null,
                null,
                null,
                null,
                null,
                null,
                null,
                null),
            List.of());
    HbcProgram program =
        new HbcProgram(
            List.of(
                "+",
                1L,
                2L,
                hara.lang.data.Vector.Standard.from(null, 1L, 2L, 3L),
                "std.protocol.icount.ICount/count",
                "std.native.Base/number?"),
            List.of(),
            List.of(entry),
            0);
    Source source =
        Source.newBuilder(
                HaraLanguage.ID,
                ByteSequence.create(HbcCodec.encode(program)),
                "registry-opcodes.hbc")
            .mimeType(HaraLanguage.BYTECODE_MIME_TYPE)
            .build();
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals("[3 3 true 3]", context.eval(source).toString());
    }
  }

  @Test
  public void concatListMaterializesSyntaxQuoteSplices() throws Exception {
    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            0,
            2,
            List.of(
                new Instruction(Opcode.CONSTANT, 0, 0, 0),
                new Instruction(Opcode.CONSTANT, 1, 0, 0),
                new Instruction(Opcode.CONCAT_LIST, 2, 0, 0),
                Instruction.of(Opcode.RETURN)),
            Arrays.asList(null, null, null, null),
            List.of());
    HbcProgram program =
        new HbcProgram(
            List.of(
                hara.lang.data.Vector.Standard.from(null, 1L, 2L),
                hara.lang.data.List.Standard.from(null, 3L)),
            List.of(),
            List.of(entry),
            0);
    Source source =
        Source.newBuilder(
                HaraLanguage.ID,
                ByteSequence.create(HbcCodec.encode(program)),
                "concat-list.hbc")
            .mimeType(HaraLanguage.BYTECODE_MIME_TYPE)
            .build();
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals("(1 2 3)", context.eval(source).toString());
    }
  }

  @Test
  public void decodesEveryArtifactInTheTrackedRustFoundationBundle() throws Exception {
    byte[] bundle = Files.readAllBytes(Path.of("rust/assets/std.foundation.hbx"));
    assertArrayEquals(new byte[] {'H', 'B', 'X', '0'}, Arrays.copyOf(bundle, 4));
    byte[] payload = Arrays.copyOfRange(bundle, 36, bundle.length);
    assertArrayEquals(Arrays.copyOfRange(bundle, 4, 36), MessageDigest.getInstance("SHA-256").digest(payload));
    List<HbxBundleCodec.Module> modules = HbxBundleCodec.decode(bundle);
    List<String> expectedInventory =
        Files.readAllLines(Path.of("rust/bootstrap.namespaces")).stream().sorted().toList();
    List<String> actualInventory =
        modules.stream().map(HbxBundleCodec.Module::resource).sorted().toList();
    assertEquals(expectedInventory, actualInventory);
    for (HbxBundleCodec.Module module : modules) {
      assertEquals(32, module.sourceDigest().length);
      HbcProgram decoded = HbcCodec.decode(module.artifact());
      assertEquals(module.resource(), decoded.namespace());
      assertTrue(decoded.functions().size() > 0);
      assertTrue(HbcDisassembler.disassemble(decoded).startsWith("HBC0 entry="));
    }
  }

  @Test
  public void rustTryTableCatchesThrownGuestValues() throws Exception {
    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            1,
            1,
            List.of(
                new Instruction(Opcode.CONSTANT, 0, 0, 0),
                Instruction.of(Opcode.THROW),
                new Instruction(Opcode.LOAD_LOCAL, 0, 0, 0),
                Instruction.of(Opcode.RETURN)),
            Arrays.asList(null, null, null, null),
            List.of(
                new HbcProgram.TryEntry(
                    0,
                    2,
                    0,
                    List.of(new HbcProgram.CatchEntry("Exception", 0, 2)),
                    null,
                    null,
                    null)));
    HbcProgram program = new HbcProgram(List.of("boom"), List.of(), List.of(entry), 0);
    Source source =
        Source.newBuilder(HaraLanguage.ID, ByteSequence.create(HbcCodec.encode(program)), "catch.hbc")
            .mimeType(HaraLanguage.BYTECODE_MIME_TYPE)
            .build();
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals("boom", context.eval(source).asString());
    }
  }

  @Test
  public void automaticallyLoadsEagerAndRequiredRustFoundationModules() throws Exception {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals("HARA", context.eval(HaraLanguage.ID, "(std.foundation.string/upper \"hara\")").asString());
      assertEquals(42L, context.eval(HaraLanguage.ID, "(std.foundation/if-not false 42)").asLong());
      assertEquals(
          6L,
          context
              .eval(HaraLanguage.ID, "(std.foundation/cond-> 1 true inc true (* 3))")
              .asLong());
      assertEquals(
          "[1 2]",
          context
              .eval(HaraLanguage.ID, "(do (ns hbx.referral) (vector 1 2))")
              .toString());
      assertEquals(
          "[42]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(do (require 'std.logic.kanren) "
                      + "(std.logic.kanren/run* (fn [query] (std.logic.kanren/== query 42))))")
              .toString());
      assertFalse(
          context
              .eval(
                  HaraLanguage.ID,
                  "(do (require 'code.vm.model) (ns-find 'code.vm.model))")
              .isNull());
    }
  }

  @Test
  public void hostCallAndSettledAwaitUseTheSharedHostPromiseBoundary() throws Exception {
    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            0,
            3,
            List.of(
                new Instruction(Opcode.CONSTANT, 0, 0, 0),
                new Instruction(Opcode.CONSTANT, 1, 0, 0),
                new Instruction(Opcode.BUILD_VECTOR, 0, 0, 0),
                Instruction.of(Opcode.HOST_CALL),
                Instruction.of(Opcode.AWAIT),
                Instruction.of(Opcode.RETURN)),
            Arrays.asList(null, null, null, null, null, null),
            List.of());
    HbcProgram program = new HbcProgram(List.of("host", "describe"), List.of(), List.of(entry), 0);
    Source source =
        Source.newBuilder(HaraLanguage.ID, ByteSequence.create(HbcCodec.encode(program)), "host.hbc")
            .mimeType(HaraLanguage.BYTECODE_MIME_TYPE)
            .build();
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      org.graalvm.polyglot.Value result = context.eval(source);
      assertTrue(result.hasHashEntries() || result.hasMembers());
    }
  }

  @Test
  public void portableMachineKeepsCompactTuplesBehindTheVectorSurface() throws Exception {
    List<Object> constants =
        new ArrayList<>(List.of("type", "vector?", "tuple?", "pair?"));
    for (long value = 1; value <= 9; value++) constants.add(value);

    List<Instruction> code = new ArrayList<>();
    appendVectorCall(code, 0, 0);
    appendVectorCall(code, 1, 0);
    appendVectorCall(code, 2, 0);
    appendVectorCall(code, 3, 2);
    appendVectorCall(code, 0, 8);
    appendVectorCall(code, 2, 8);
    appendVectorCall(code, 0, 9);
    appendVectorCall(code, 1, 9);
    appendVectorCall(code, 2, 9);
    code.add(new Instruction(Opcode.BUILD_VECTOR, 9, 0, 0));
    code.add(Instruction.of(Opcode.RETURN));

    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            0,
            18,
            code,
            new ArrayList<>(Collections.nCopies(code.size(), null)),
            List.of());
    HbcProgram program = new HbcProgram(constants, List.of(), List.of(entry), 0);
    Source source =
        Source.newBuilder(
                HaraLanguage.ID,
                ByteSequence.create(HbcCodec.encode(program)),
                "vector-surface.hbc")
            .mimeType(HaraLanguage.BYTECODE_MIME_TYPE)
            .build();
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[:std.native.Tuple true true true :std.native.Tuple true :std.native.Vector true false]",
          context.eval(source).toString());
    }
  }

  private static void appendVectorCall(
      List<Instruction> code, int builtinConstant, int vectorCount) {
    code.add(new Instruction(Opcode.BUILTIN_VALUE, builtinConstant, 0, 0));
    for (int index = 0; index < vectorCount; index++) {
      code.add(new Instruction(Opcode.CONSTANT, 4 + index, 0, 0));
    }
    code.add(new Instruction(Opcode.BUILD_VECTOR, vectorCount, 0, 0));
    code.add(new Instruction(Opcode.CALL, 1, 0, 0));
  }

  @Test
  public void asyncBytecodeFunctionsReturnPromisesThatAwaitToTheirValue() throws Exception {
    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            0,
            1,
            List.of(
                new Instruction(Opcode.CLOSURE, 1, 0, 0),
                new Instruction(Opcode.CALL, 0, 0, 0),
                Instruction.of(Opcode.AWAIT),
                Instruction.of(Opcode.RETURN)),
            Arrays.asList(null, null, null, null),
            List.of());
    Function async =
        new Function(
            "answer",
            true,
            0,
            false,
            0,
            0,
            1,
            List.of(new Instruction(Opcode.CONSTANT, 0, 0, 0), Instruction.of(Opcode.RETURN)),
            Arrays.asList(null, null),
            List.of());
    HbcProgram program = new HbcProgram(List.of(42L), List.of(), List.of(entry, async), 0);
    Source source =
        Source.newBuilder(HaraLanguage.ID, ByteSequence.create(HbcCodec.encode(program)), "async.hbc")
            .mimeType(HaraLanguage.BYTECODE_MIME_TYPE)
            .build();
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(42L, context.eval(source).asLong());
    }
  }

  @Test
  public void staticBytecodeRecursionDoesNotConsumeTheJavaStack() throws Exception {
    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            0,
            1,
            List.of(
                new Instruction(Opcode.CONSTANT, 2, 0, 0),
                new Instruction(Opcode.CALL_STATIC, 1, 1, 0),
                Instruction.of(Opcode.RETURN)),
            Arrays.asList(null, null, null),
            List.of());
    Function recursive =
        new Function(
            "count-down",
            false,
            1,
            false,
            0,
            1,
            2,
            List.of(
                new Instruction(Opcode.LOAD_LOCAL, 0, 0, 0),
                new Instruction(Opcode.CONSTANT, 0, 0, 0),
                new Instruction(Opcode.PRIMITIVE, Primitive.LESS.id(), 2, 0),
                new Instruction(Opcode.JUMP_IF_FALSE, 6, 0, 0),
                new Instruction(Opcode.CONSTANT, 1, 0, 0),
                Instruction.of(Opcode.RETURN),
                new Instruction(Opcode.LOAD_LOCAL, 0, 0, 0),
                new Instruction(Opcode.CONSTANT, 0, 0, 0),
                new Instruction(Opcode.PRIMITIVE, Primitive.SUBTRACT.id(), 2, 0),
                new Instruction(Opcode.CALL_STATIC, 1, 1, 0),
                Instruction.of(Opcode.RETURN)),
            Arrays.asList(null, null, null, null, null, null, null, null, null, null, null),
            List.of());
    HbcProgram program =
        new HbcProgram(List.of(1L, 0L, 10_000L), List.of(), List.of(entry, recursive), 0);
    Source source =
        Source.newBuilder(HaraLanguage.ID, ByteSequence.create(HbcCodec.encode(program)), "deep.hbc")
            .mimeType(HaraLanguage.BYTECODE_MIME_TYPE)
            .build();
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(0L, context.eval(source).asLong());
    }
  }

  @Test
  public void exceptionsUnwindAcrossExplicitBytecodeCallFrames() throws Exception {
    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            1,
            1,
            List.of(
                new Instruction(Opcode.CALL_STATIC, 1, 0, 0),
                Instruction.of(Opcode.RETURN),
                new Instruction(Opcode.LOAD_LOCAL, 0, 0, 0),
                Instruction.of(Opcode.RETURN)),
            Arrays.asList(null, null, null, null),
            List.of(
                new HbcProgram.TryEntry(
                    0,
                    1,
                    0,
                    List.of(new HbcProgram.CatchEntry("Exception", 0, 2)),
                    null,
                    null,
                    null)));
    Function throwing =
        new Function(
            "throwing",
            false,
            0,
            false,
            0,
            0,
            1,
            List.of(new Instruction(Opcode.CONSTANT, 0, 0, 0), Instruction.of(Opcode.THROW)),
            Arrays.asList(null, null),
            List.of());
    HbcProgram program = new HbcProgram(List.of(42L), List.of(), List.of(entry, throwing), 0);
    Source source =
        Source.newBuilder(HaraLanguage.ID, ByteSequence.create(HbcCodec.encode(program)), "unwind.hbc")
            .mimeType(HaraLanguage.BYTECODE_MIME_TYPE)
            .build();
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(42L, context.eval(source).asLong());
    }
  }

  @Test
  public void defGlobalPreservesRustArtifactMetadata() throws Exception {
    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            0,
            1,
            List.of(
                new Instruction(Opcode.CONSTANT, 0, 0, 0),
                new Instruction(Opcode.DEF_GLOBAL, 1, 0, 0),
                Instruction.of(Opcode.RETURN)),
            Arrays.asList(null, null, null),
            List.of());
    HbcProgram.MetadataValue doc =
        new HbcProgram.MetadataValue(HbcProgram.MetadataValue.Kind.KEYWORD, hara.lang.data.Keyword.create("doc"));
    HbcProgram.MetadataValue text =
        new HbcProgram.MetadataValue(HbcProgram.MetadataValue.Kind.STRING, "portable metadata");
    HbcProgram program =
        new HbcProgram(
            List.of(42L, "answer"),
            List.of(List.of(new HbcProgram.MetadataEntry(doc, text))),
            List.of(entry),
            0);
    Source source =
        Source.newBuilder(HaraLanguage.ID, ByteSequence.create(HbcCodec.encode(program)), "meta.hbc")
            .mimeType(HaraLanguage.BYTECODE_MIME_TYPE)
            .build();
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(source);
      assertEquals(
          "portable metadata",
          context.eval(HaraLanguage.ID, "(get (meta #'answer) :doc)").asString());
    }
  }

  @Test
  public void executesEveryRustProducedDisplayConformanceArtifact() throws Exception {
    List<HbcConformanceCorpus.Case> cases =
        HbcConformanceCorpus.decode(
            Files.readAllBytes(Path.of("rust/assets/bytecode-conformance.hcc")));
    assertTrue(cases.size() >= 80);
    for (HbcConformanceCorpus.Case testCase : cases) {
      Source source =
          Source.newBuilder(
                  HaraLanguage.ID,
                  ByteSequence.create(testCase.artifact()),
                  testCase.id() + ".hbc")
              .mimeType(HaraLanguage.BYTECODE_MIME_TYPE)
              .build();
      try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
        org.graalvm.polyglot.Value actual;
        try {
          actual = context.eval(source);
        } catch (RuntimeException failure) {
          throw new AssertionError(
              testCase.id()
                  + "\nconstants="
                  + HbcCodec.decode(testCase.artifact()).constants()
                  + "\n"
                  + HbcDisassembler.disassemble(HbcCodec.decode(testCase.artifact())),
              failure);
        }
        String display =
            actual.isNull()
                ? "nil"
                : actual.isString() ? G.display(actual.asString()) : actual.toString();
        assertEquals(testCase.id(), testCase.expectedDisplay(), display);
      }
    }
  }

  private static byte[] takeBundleField(ByteBuffer input) {
    int size = input.getInt();
    byte[] value = new byte[size];
    input.get(value);
    return value;
  }

  private static HbcProgram arithmeticProgram() {
    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            0,
            2,
            List.of(
                new Instruction(Opcode.CONSTANT, 0, 0, 0),
                new Instruction(Opcode.CONSTANT, 1, 0, 0),
                new Instruction(Opcode.PRIMITIVE, Primitive.ADD.id(), 2, 0),
                Instruction.of(Opcode.RETURN)),
            Arrays.asList(null, null, null, null),
            List.of());
    return new HbcProgram(List.of(19L, 23L), List.of(), List.of(entry), 0);
  }
}
