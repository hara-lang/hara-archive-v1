package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.HexFormat;
import java.util.Set;
import org.junit.Test;

/** Proves the host-only WebDAV artifact is selected and loaded through the JVM package boundary. */
public class WebdavPackagePilotTest {
  @Test
  public void prebuiltProviderJarRegistersThroughVerifiedJvmLoader() throws Exception {
    String configured = System.getProperty("hara.webdav.provider.jar");
    if (configured == null || configured.isBlank()) {
      throw new AssertionError("hara.webdav.provider.jar must point at the provider JAR");
    }
    Path artifact = Path.of(configured).toAbsolutePath().normalize();
    assertTrue(Files.isRegularFile(artifact));
    String digest =
        "sha256:"
            + HexFormat.of()
                .formatHex(MessageDigest.getInstance("SHA-256").digest(Files.readAllBytes(artifact)));

    FilesystemProviderRegistry registry = new FilesystemProviderRegistry();
    JvmPackageLoader.Selection selection =
        new JvmPackageLoader.Selection(
            "hara:hara/filesystem-webdav",
            artifact,
            digest,
            JvmPackageProvider.ABI,
            "hara.provider.webdav.WebdavPackageProvider",
            Set.of());

    try (JvmPackageLoader.LoadedProvider loaded = JvmPackageLoader.load(selection, registry)) {
      assertEquals("hara:hara/filesystem-webdav", loaded.identity());
      assertTrue(registry.contains("webdav"));
    }
    assertFalse(registry.contains("webdav"));
  }
}
