package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.nio.file.Path;
import org.junit.Test;

public class HaraExtensionManifestTest {
  @Test
  public void packagedAnswer42ManifestMatchesTheProviderContract() throws Exception {
    Path descriptor = Path.of("lib/examples/extensions/demo/000-answer-42/project.edn");
    HaraProject project = HaraProject.read(descriptor);
    HaraExtensionManifest manifest =
        HaraExtensionManifest.parse(
            project.extensionManifestSource("demo.000-answer-42"), descriptor.toString());
    assertEquals("demo.000-answer-42", manifest.namespace());
    assertEquals("0.1.0", manifest.version());
    assertEquals("wasm", manifest.provider());
    assertEquals("answer-42.wasm", manifest.module());
    assertEquals("core.v1", manifest.abi());
    assertEquals(2, manifest.exports().size());
    assertEquals("version", manifest.exports().get("version").wasmExport());
    assertEquals("i32", manifest.exports().get("version").returns());
    assertEquals(2, manifest.exports().get("add").arguments().size());
    assertTrue(manifest.capabilities().isEmpty());
  }

  @Test
  public void parsesPortableRawWasmExportAliases() {
    String source =
        "{:namespace \"codec.echo\" :version \"1\" :provider :wasm "
            + ":module \"echo.wasm\" :abi :memory.v1 "
            + ":exports {\"echo\" {:wasm/export \"echo_bytes\" "
            + ":args [:bytes] :returns :bytes :async false}} "
            + ":capabilities [] :assets [\"bindings.edn\"]}";
    HaraExtensionManifest manifest = HaraExtensionManifest.parse(source, "test");
    assertEquals("echo_bytes", manifest.exports().get("echo").wasmExport());
    assertThrows(
        IllegalArgumentException.class,
        () -> HaraExtensionManifest.parse(source.replace(":wasm/export", ":other/export"), "test"));
  }

  @Test
  public void parsesCompactPublicHandleTags() {
    String source =
        "{:namespace \"math.tensor\" :identity \"hara/math.tensor\" :version \"1\" :provider :wasm "
            + ":module \"tensor.wasm\" :abi :hta.v1 "
            + ":exports {\"open\" {:args [] :returns :value :async true}} "
            + ":handles {\"tensor\" {:tag math}} :capabilities []}";
    HaraExtensionManifest manifest = HaraExtensionManifest.parse(source, "test");
    assertEquals("hara/math.tensor", manifest.identity());
    assertEquals("math", manifest.handleTag("tensor"));
    assertEquals(null, manifest.handleTag("buffer"));
    assertThrows(
        IllegalArgumentException.class,
        () -> HaraExtensionManifest.parse(source.replace(":tag math", ":tag Math"), "test"));
    assertThrows(
        IllegalArgumentException.class,
        () ->
            HaraExtensionManifest.parse(
                source.replace("hara/math.tensor", "Hara/math.tensor"), "test"));
  }

  @Test
  public void malformedManifestsFailBeforeProviderSelection() {
    String base =
        "{:namespace \"demo.extension\" :version \"1\" :provider :wasm "
            + ":module \"demo.wasm\" :abi :core.v1 "
            + ":exports {\"run\" {:args [] :returns :i32}} "
            + ":capabilities []}";
    assertThrows(
        IllegalArgumentException.class,
        () ->
            HaraExtensionManifest.parse(
                base.replace(":provider :wasm", ":provider \"wasm\""), "test"));
    assertThrows(
        IllegalArgumentException.class,
        () ->
            HaraExtensionManifest.parse(
                base.replace(":capabilities []", ":capabilities [] :extra true"), "test"));
  }

  @Test
  public void htaTargetsNameProviderImplementationsNotWorkers() {
    String source =
        "{:namespace \"demo.hta\" :version \"1\" :provider :hta :abi :hta.v1 "
            + ":targets {:node {:provider \"node/provider.mjs\" :runtime :process} "
            + ":browser {:provider \"browser/provider.mjs\" :runtime :web-worker}} "
            + ":exports {\"open\" {:args [] :returns :value}} :capabilities []}";
    HaraExtensionManifest manifest = HaraExtensionManifest.parse(source, "test");
    assertEquals("browser/provider.mjs", manifest.target("browser").provider());
    assertThrows(
        IllegalArgumentException.class,
        () -> HaraExtensionManifest.parse(source.replace(":provider \"browser/provider.mjs\"", ":module \"browser/worker.mjs\""), "test"));
  }
}
