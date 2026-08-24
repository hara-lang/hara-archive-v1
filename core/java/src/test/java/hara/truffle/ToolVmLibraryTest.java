package hara.truffle;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import hara.lang.data.Keyword;
import hara.lang.protocol.IMapType;
import hara.truffle.bytecode.HbcCodec;
import hara.truffle.bytecode.HbcProgram;
import java.nio.charset.StandardCharsets;
import java.util.List;
import java.util.ServiceLoader;
import java.util.Set;
import java.util.stream.Collectors;
import java.util.stream.StreamSupport;
import org.graalvm.polyglot.Context;
import org.junit.Test;

public class ToolVmLibraryTest {
  @Test
  public void providerIsDiscoverableAndPublicFacadeLoads() {
    Set<String> namespaces =
        StreamSupport.stream(ServiceLoader.load(HaraLibraryProvider.class).spliterator(), false)
            .map(HaraLibraryProvider::namespace)
            .collect(Collectors.toSet());
    assertTrue(namespaces.contains("tool.vm.provider"));

    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          ":truffle",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns tool.vm.provider-probe (:require [tool.vm :as vm])) "
                      + "(:provider/id (vm/current-provider))")
              .toString());
    }
  }

  @Test
  public void publicFacadeExecutesCanonicalHalcThroughAstLowering() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          42L,
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns tool.vm.execute-probe (:require [tool.vm :as vm])) "
                      + "(vm/execute "
                      + " (vm/transform \"(ns sample.execute) (+ 19 23)\" :halc)"
                      + " {:provider :truffle})")
              .asLong());
    }
  }

  @Test
  public void failedHalcExecutionRollsBackNamespaceMutation() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns tool.vm.rollback-probe (:require [tool.vm :as vm])) "
                      + "(let [artifact (vm/transform "
                      + "\"(ns sample.rollback) (def leaked 1) (/ 1 0)\" :halc)] "
                      + " (try (vm/execute artifact) (catch Throwable error nil)) "
                      + " (= nil (resolve 'sample.rollback/leaked)))")
              .asBoolean());
    }
  }

  @Test
  public void halcValidationAndInspectionUseCanonicalCodec() {
    String source = "(ns sample.vm) (def value 42)";
    Object[] forms = HaraLanguage.readAll(source, "sample/vm.hal");
    byte[] artifact =
        HalcArtifact.encode(
            "sample.vm",
            "sample/vm.hal",
            source.getBytes(StandardCharsets.UTF_8),
            forms);

    assertEquals(
        Boolean.TRUE,
        ToolVmLibrary.validate(null, new Object[] {Keyword.create("halc"), artifact}));
    IMapType<Keyword, Object> inspection =
        (IMapType<Keyword, Object>) ToolVmLibrary.inspect(null, new Object[] {Keyword.create("halc"), artifact});
    assertEquals(Keyword.create("halc"), inspection.lookup(Keyword.create("artifact/format")));
    assertEquals("sample.vm", inspection.lookup(Keyword.create("module/namespace")));
    assertEquals(2L, inspection.lookup(Keyword.create("forms/count")));
  }

  @Test
  public void halToHalcTransformUsesCanonicalEncoder() {
    String source = "(ns tool.vm.parity)\n(def answer (+ 19 23))\n";
    byte[] artifact =
        (byte[])
            ToolVmLibrary.transform(
                null,
                new Object[] {
                  Keyword.create("hal"),
                  Keyword.create("halc"),
                  source,
                  hara.lang.data.OrderedMap.Standard.from(
                      null,
                      new Object[] {Keyword.create("resource"), "fixtures/tool-vm-parity.hal"})
                });

    assertEquals(
        Boolean.TRUE,
        ToolVmLibrary.validate(null, new Object[] {Keyword.create("halc"), artifact}));
    assertEquals("tool.vm.parity", HalcArtifact.decode(artifact).namespace);
    assertEquals("fixtures/tool-vm-parity.hal", HalcArtifact.decode(artifact).resource);
    assertArrayEquals(
        java.util.HexFormat.of()
            .parseHex(
                "48414c4300010001000000b6865eb5f9ac7dc1198d8a6345686ffa257eee"
                    + "e47e15bd45418766fa26e8edaa520000000e746f6f6c2e766d2e70617269"
                    + "74790000001b66697874757265732f746f6f6c2d766d2d7061726974792e"
                    + "68616c25e7e2e6fedd97d111cd6f9554c8d4bf51e11dbdcf5746d4f35c27"
                    + "f96198d598000000020b000000020900000000026e730009000000000e74"
                    + "6f6f6c2e766d2e70617269747900000b0000000309000000000364656600"
                    + "090000000006616e73776572000b000000030900000000012b0003000000"
                    + "00000000130300000000000000170000"),
        artifact);
  }

  @Test
  public void rustProducedHbcValidatesInTruffle() {
    byte[] artifact =
        java.util.HexFormat.of()
            .parseHex(
                "484243300000005f0000010000000475736572000000010000000d4854413003"
                    + "000000000000002a000000000000000000000000000000000000000100000000"
                    + "0000000000000100000002000000000018000000020100000000000000010000"
                    + "000100000000006073811fa3086d8edff969b6f31169f2d358937b295630863e"
                    + "c63366450debec");

    assertEquals(
        Boolean.TRUE,
        ToolVmLibrary.validate(null, new Object[] {Keyword.create("hbc"), artifact}));
    assertTrue(
        ToolVmLibrary.disassemble(null, new Object[] {artifact}).toString().contains("const 0 42"));
  }

  @Test
  public void hbcValidationInspectionAndDisassemblyUseCanonicalCodec() {
    HbcProgram.Function function =
        new HbcProgram.Function(
            "entry",
            false,
            0,
            false,
            0,
            0,
            2,
            List.of(
                new HbcProgram.Instruction(HbcProgram.Opcode.CONSTANT, 0, 0, 0),
                new HbcProgram.Instruction(HbcProgram.Opcode.CONSTANT, 1, 0, 0),
                new HbcProgram.Instruction(
                    HbcProgram.Opcode.PRIMITIVE, HbcProgram.Primitive.ADD.id(), 2, 0),
                new HbcProgram.Instruction(HbcProgram.Opcode.RETURN, 0, 0, 0)),
            java.util.Arrays.asList(null, null, null, null),
            List.of());
    HbcProgram program =
        new HbcProgram(List.of(19L, 23L), List.of(), List.of(function), 0);
    byte[] artifact = HbcCodec.encode(program);

    assertEquals(
        Boolean.TRUE,
        ToolVmLibrary.validate(null, new Object[] {Keyword.create("hbc"), artifact}));
    IMapType<Keyword, Object> inspection =
        (IMapType<Keyword, Object>) ToolVmLibrary.inspect(null, new Object[] {Keyword.create("hbc"), artifact});
    assertEquals(Keyword.create("hbc"), inspection.lookup(Keyword.create("artifact/format")));
    assertEquals(1L, inspection.lookup(Keyword.create("functions/count")));
    assertTrue(ToolVmLibrary.disassemble(null, new Object[] {artifact}).toString().startsWith("HBC0 entry="));
  }
}
