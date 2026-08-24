package hara.truffle;

import hara.kernel.NativeMode;
import hara.kernel.maven.MavenResolver;
import java.io.File;
import java.io.IOException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Locale;
import java.util.stream.Stream;
import javax.tools.Diagnostic;
import javax.tools.DiagnosticCollector;
import javax.tools.JavaCompiler;
import javax.tools.JavaFileObject;
import javax.tools.StandardJavaFileManager;
import javax.tools.ToolProvider;
import java.util.jar.JarEntry;
import java.util.jar.JarOutputStream;
import org.eclipse.aether.resolution.DependencyResolutionException;

/** Prepares the Maven graph and Java boundary code for one Hara project invocation. */
final class HaraJvmProject implements AutoCloseable {
  record BuildResult(Path artifact, List<Path> dependencies, int compiledSources) {
    BuildResult {
      artifact = artifact.toAbsolutePath().normalize();
      dependencies = List.copyOf(dependencies);
    }
  }

  private final ClassLoader previousLoader;
  private final URLClassLoader projectLoader;
  private final List<Path> artifacts;
  private final int compiledSources;

  private HaraJvmProject(
      ClassLoader previousLoader,
      URLClassLoader projectLoader,
      List<Path> artifacts,
      int compiledSources) {
    this.previousLoader = previousLoader;
    this.projectLoader = projectLoader;
    this.artifacts = List.copyOf(artifacts);
    this.compiledSources = compiledSources;
  }

  static HaraJvmProject prepare(HaraProject project, boolean offline) {
    List<Path> artifacts = resolveDependencies(project, offline);
    int compiledSources = compileJava(project, artifacts);
    ArrayList<URL> urls = new ArrayList<>();
    try {
      if (Files.isDirectory(project.jvmTargetPath())) {
        urls.add(project.jvmTargetPath().toUri().toURL());
      }
      for (Path artifact : artifacts) urls.add(artifact.toUri().toURL());
    } catch (IOException error) {
      throw new HaraException("Unable to build JVM project classpath: " + error.getMessage());
    }
    ClassLoader previous = Thread.currentThread().getContextClassLoader();
    ClassLoader parent = previous == null ? HaraJvmProject.class.getClassLoader() : previous;
    URLClassLoader loader = new URLClassLoader(urls.toArray(URL[]::new), parent);
    Thread.currentThread().setContextClassLoader(loader);
    return new HaraJvmProject(previous, loader, artifacts, compiledSources);
  }

  static BuildResult buildPackage(HaraProject project, boolean offline) {
    if (project.jvmEntryPoint() == null) return null;
    List<Path> artifacts = resolveDependencies(project, offline);
    int compiledSources = compileJava(project, artifacts);
    if (compiledSources == 0) return null;
    Path entryPoint =
        project
            .jvmTargetPath()
            .resolve(project.jvmEntryPoint().replace('.', java.io.File.separatorChar) + ".class")
            .normalize();
    if (!entryPoint.startsWith(project.jvmTargetPath()) || !Files.isRegularFile(entryPoint)) {
      throw new HaraException(
          "JVM package entry point was not compiled: " + project.jvmEntryPoint());
    }
    Path artifact =
        project
            .jvmTargetPath()
            .resolveSibling(
                project.name().display().replace('/', '-')
                    + "-"
                    + project.version()
                    + ".jar");
    writeJar(project.jvmTargetPath(), artifact);
    return new BuildResult(artifact, artifacts, compiledSources);
  }

  static List<Path> resolveDependencies(HaraProject project, boolean offline) {
    if (project.jvmDependencies().isEmpty()) return List.of();
    NativeMode.requireDisabled("project Maven dependency resolution");
    List<String> coordinates =
        project.jvmDependencies().stream().map(HaraProject.JvmDependency::coordinate).toList();
    try {
      return new MavenResolver(offline).resolve(coordinates).stream()
          .map(File::toPath)
          .map(path -> path.toAbsolutePath().normalize())
          .toList();
    } catch (DependencyResolutionException error) {
      String mode = offline ? " in offline mode" : "";
      throw new HaraException(
          "Unable to resolve JVM dependencies" + mode + ": " + resolutionMessage(error));
    }
  }

  List<Path> artifacts() {
    return artifacts;
  }

  int compiledSources() {
    return compiledSources;
  }

  @Override
  public void close() {
    Thread.currentThread().setContextClassLoader(previousLoader);
    try {
      projectLoader.close();
    } catch (IOException error) {
      throw new HaraException("Unable to close JVM project classpath: " + error.getMessage());
    }
  }

  private static int compileJava(HaraProject project, List<Path> artifacts) {
    ArrayList<Path> sources = new ArrayList<>();
    for (Path sourceRoot : project.jvmSourcePaths()) {
      if (!Files.isDirectory(sourceRoot)) continue;
      try (Stream<Path> paths = Files.walk(sourceRoot)) {
        paths
            .filter(path -> path.toString().endsWith(".java"))
            .sorted(Comparator.naturalOrder())
            .forEach(sources::add);
      } catch (IOException error) {
        throw new HaraException("Unable to scan JVM sources under " + sourceRoot + ": " + error.getMessage());
      }
    }
    if (sources.isEmpty()) return 0;
    NativeMode.requireDisabled("project Java compilation");
    JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
    if (compiler == null) {
      throw new HaraException("JVM project compilation requires a JDK; no Java compiler is available");
    }
    try {
      Files.createDirectories(project.jvmTargetPath());
    } catch (IOException error) {
      throw new HaraException(
          "Unable to create JVM target path " + project.jvmTargetPath() + ": " + error.getMessage());
    }
    DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
    try (StandardJavaFileManager files =
        compiler.getStandardFileManager(diagnostics, Locale.ROOT, StandardCharsets.UTF_8)) {
      Iterable<? extends JavaFileObject> units =
          files.getJavaFileObjectsFromPaths(sources);
      ArrayList<String> options = new ArrayList<>();
      options.addAll(List.of("--release", "21", "-parameters", "-d", project.jvmTargetPath().toString()));
      String classpath = compilerClasspath(artifacts);
      if (!classpath.isBlank()) options.addAll(List.of("-classpath", classpath));
      Boolean success = compiler.getTask(null, files, diagnostics, options, null, units).call();
      if (!Boolean.TRUE.equals(success)) {
        throw new HaraException("JVM source compilation failed:\n" + diagnosticText(diagnostics));
      }
      return sources.size();
    } catch (IOException error) {
      throw new HaraException("Unable to compile JVM sources: " + error.getMessage());
    }
  }

  private static void writeJar(Path classes, Path artifact) {
    try {
      if (artifact.getParent() != null) Files.createDirectories(artifact.getParent());
      try (JarOutputStream output = new JarOutputStream(Files.newOutputStream(artifact))) {
        try (Stream<Path> paths = Files.walk(classes)) {
          for (Path path : paths.filter(Files::isRegularFile).sorted().toList()) {
            String name = classes.relativize(path).toString().replace('\\', '/');
            JarEntry entry = new JarEntry(name);
            entry.setTime(0L);
            output.putNextEntry(entry);
            Files.copy(path, output);
            output.closeEntry();
          }
        }
      }
    } catch (IOException error) {
      throw new HaraException("Unable to build JVM package JAR: " + error.getMessage());
    }
  }

  private static String compilerClasspath(List<Path> artifacts) {
    ArrayList<String> entries = new ArrayList<>();
    String current = System.getProperty("java.class.path", "");
    if (!current.isBlank()) entries.add(current);
    artifacts.stream().map(Path::toString).forEach(entries::add);
    return String.join(File.pathSeparator, entries);
  }

  private static String diagnosticText(DiagnosticCollector<JavaFileObject> diagnostics) {
    StringBuilder text = new StringBuilder();
    for (Diagnostic<? extends JavaFileObject> diagnostic : diagnostics.getDiagnostics()) {
      if (text.length() > 0) text.append('\n');
      if (diagnostic.getSource() != null) text.append(diagnostic.getSource().getName()).append(':');
      if (diagnostic.getLineNumber() >= 0) text.append(diagnostic.getLineNumber()).append(':');
      text.append(' ').append(diagnostic.getMessage(Locale.ROOT));
    }
    return text.toString();
  }

  private static String resolutionMessage(DependencyResolutionException error) {
    Throwable cause = error;
    while (cause.getCause() != null) cause = cause.getCause();
    String message = cause.getMessage();
    return message == null || message.isBlank() ? error.getMessage() : message;
  }
}
