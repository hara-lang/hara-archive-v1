package hara.kernel.base;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.kernel.NativeMode;
import hara.kernel.flavor.NativeCapability;
import hara.kernel.flavor.NativeFlavorException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.EnumSet;
import org.junit.Test;

public class JvmFlavorLibrariesTest {
  @Test
  public void exposesReflectionThroughTheExplicitJvmNamespace() {
    RT.Instance<Object> runtime = runtime(EnumSet.of(NativeCapability.REFLECTION));

    assertEquals(
        "java.lang.String",
        runtime.eval(runtime.readString("(hara.native.jvm.reflect/name String)")));
    assertEquals(
        Boolean.TRUE,
        runtime.eval(
            runtime.readString(
                "(hara.native.jvm.reflect/instance? String (new String \"value\"))")));

    String[] fields =
        (String[]) runtime.eval(runtime.readString("(hara.native.jvm.reflect/fields Point)"));
    assertTrue(Arrays.asList(fields).contains("x"));
    assertTrue(runtime.currentSymbolNames().contains("String/valueOf"));
    assertTrue(runtime.currentSymbolNames().contains("hara.native.jvm.reflect/instance?"));
  }

  @Test
  public void classpathRequiresAnIndependentGrant() {
    RT.Instance<Object> runtime = runtime(EnumSet.of(NativeCapability.REFLECTION));

    NativeFlavorException classpath =
        assertThrows(
            NativeFlavorException.class,
            () -> runtime.eval(runtime.readString("(hara.native.jvm.classpath/paths)")));
    assertEquals(NativeFlavorException.Kind.DENIED, classpath.kind());

  }

  @Test
  public void grantedClasspathServicesAreUsable() throws Exception {
    RT.Instance<Object> runtime = runtime(EnumSet.allOf(NativeCapability.class));
    Path directory = Files.createTempDirectory("hara-jvm-classpath-");
    try {
      String escaped = directory.toString().replace("\\", "\\\\").replace("\"", "\\\"");
      String added =
          (String)
              runtime.eval(
                  runtime.readString("(hara.native.jvm.classpath/add \"" + escaped + "\")"));
      assertTrue(added.startsWith("file:"));

    } finally {
      Files.deleteIfExists(directory);
    }
  }

  @Test
  public void nativeModeReportsDynamicJvmServicesAsUnavailable() {
    String previous = System.getProperty(NativeMode.PROPERTY);
    try {
      System.setProperty(NativeMode.PROPERTY, "true");
      RT.Instance<Object> runtime = runtime(EnumSet.allOf(NativeCapability.class));
      NativeFlavorException error =
          assertThrows(
              NativeFlavorException.class,
              () -> runtime.eval(runtime.readString("(hara.native.jvm.classpath/paths)")));
      assertEquals(NativeFlavorException.Kind.UNSUPPORTED, error.kind());
      assertTrue(error.getMessage().contains("Native mode does not support classpath inspection"));
    } finally {
      if (previous == null) {
        System.clearProperty(NativeMode.PROPERTY);
      } else {
        System.setProperty(NativeMode.PROPERTY, previous);
      }
    }
  }

  private static RT.Instance<Object> runtime(EnumSet<NativeCapability> capabilities) {
    RT.Instance<Object> runtime = new RT.Instance<>(null, "jvm-libraries-test", capabilities);
    runtime.eval(
        runtime.readString(
            "(ns jvm-libraries-test (:flavor :jvm [java.lang String] [java.awt Point]))"));
    return runtime;
  }
}
