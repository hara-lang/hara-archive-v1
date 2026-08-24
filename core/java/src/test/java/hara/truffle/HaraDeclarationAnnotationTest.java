package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertTrue;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import javax.annotation.processing.AbstractProcessor;
import javax.annotation.processing.RoundEnvironment;
import javax.annotation.processing.SupportedAnnotationTypes;
import javax.lang.model.SourceVersion;
import javax.lang.model.element.AnnotationMirror;
import javax.lang.model.element.AnnotationValue;
import javax.lang.model.element.Element;
import javax.lang.model.element.ExecutableElement;
import javax.lang.model.element.TypeElement;
import javax.tools.Diagnostic;
import javax.tools.JavaCompiler;
import javax.tools.JavaFileObject;
import javax.tools.StandardJavaFileManager;
import javax.tools.ToolProvider;
import org.junit.Test;

/** Verifies that declaration annotations are consumable by a compile-time processor. */
public class HaraDeclarationAnnotationTest {
  @Test
  public void protocolAndNativeAnnotationsExposeStableDeclarationMetadata() throws Exception {
    JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
    assertNotNull("JDK compiler is required for annotation processing", compiler);

    Path protocol =
        Path.of("java/src/main/java/hara/lang/protocol/IAssoc.java").toAbsolutePath();
    Path nativeProvider =
        Path.of("java/src/main/java/hara/truffle/BytesLibraryProvider.java").toAbsolutePath();
    assertTrue("protocol source is missing: " + protocol, Files.isRegularFile(protocol));
    assertTrue("native provider source is missing: " + nativeProvider, Files.isRegularFile(nativeProvider));

    DeclarationProcessor processor = new DeclarationProcessor();
    try (StandardJavaFileManager files =
        compiler.getStandardFileManager(null, null, null)) {
      Iterable<? extends JavaFileObject> units =
          files.getJavaFileObjects(protocol.toFile(), nativeProvider.toFile());
      JavaCompiler.CompilationTask task =
          compiler.getTask(
              null,
              files,
              diagnostic -> {
                if (diagnostic.getKind() == Diagnostic.Kind.ERROR) {
                  throw new AssertionError(diagnostic.toString());
                }
              },
              List.of(
                  "-proc:only",
                  "-classpath",
                  System.getProperty("java.class.path"),
                  "-sourcepath",
                  protocol.getParent().getParent().getParent().getParent().toString()),
              null,
              units);
      task.setProcessors(List.of(processor));
      assertTrue("declaration annotation processing failed", task.call());
    }

    assertEquals("std.protocol.iassoc", processor.protocol.get("namespace"));
    assertEquals("IAssoc", processor.protocol.get("name"));
    assertEquals("assoc", processor.method.get("name"));
    assertEquals("3", processor.method.get("arity"));
    assertEquals("std.native", processor.nativeBinding.get("namespace"));
    assertEquals("Bytes", processor.nativeBinding.get("name"));
  }

  @SupportedAnnotationTypes({
    "hara.lang.declaration.HaraMethod",
    "hara.lang.declaration.HaraNativeBinding",
    "hara.lang.declaration.HaraProtocolBinding"
  })
  private static final class DeclarationProcessor extends AbstractProcessor {
    private final Map<String, String> protocol = new HashMap<>();
    private final Map<String, String> method = new HashMap<>();
    private final Map<String, String> nativeBinding = new HashMap<>();

    @Override
    public SourceVersion getSupportedSourceVersion() {
      return SourceVersion.latestSupported();
    }

    @Override
    public boolean process(
        Set<? extends TypeElement> annotations, RoundEnvironment roundEnvironment) {
      for (Element element :
          roundEnvironment.getElementsAnnotatedWith(
              processingEnv.getElementUtils().getTypeElement("hara.lang.declaration.HaraProtocolBinding"))) {
        protocol.putAll(annotationValues(element, "HaraProtocolBinding"));
      }
      for (Element element :
          roundEnvironment.getElementsAnnotatedWith(
              processingEnv.getElementUtils().getTypeElement("hara.lang.declaration.HaraMethod"))) {
        method.putAll(annotationValues(element, "HaraMethod"));
        if (element instanceof ExecutableElement executable) {
          method.put("arity", Integer.toString(executable.getParameters().size() + 1));
        }
      }
      for (Element element :
          roundEnvironment.getElementsAnnotatedWith(
              processingEnv.getElementUtils().getTypeElement("hara.lang.declaration.HaraNativeBinding"))) {
        nativeBinding.putAll(annotationValues(element, "HaraNativeBinding"));
      }
      return false;
    }

    private static Map<String, String> annotationValues(Element element, String simpleName) {
      for (AnnotationMirror annotation : element.getAnnotationMirrors()) {
        if (!annotation.getAnnotationType().asElement().getSimpleName().contentEquals(simpleName)) {
          continue;
        }
        Map<String, String> values = new HashMap<>();
        for (Map.Entry<? extends ExecutableElement, ? extends AnnotationValue> entry :
            annotation.getElementValues().entrySet()) {
          values.put(
              entry.getKey().getSimpleName().toString(), entry.getValue().getValue().toString());
        }
        return values;
      }
      throw new AssertionError("missing @" + simpleName + " on " + element);
    }
  }
}
