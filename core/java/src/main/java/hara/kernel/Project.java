package hara.kernel;

import hara.kernel.base.Parser;
import hara.kernel.base.RT;
import hara.kernel.maven.Maven;
import hara.lang.base.Ex;
import hara.lang.data.Keyword;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.ILookup;

import java.io.File;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.Iterator;

public class Project {

  private static final String PROJECT_FILE = "project.hara";
  private static final String SRC_DIR = "src";
  private static final String MAIN_FILE = "main.hal";

  public static void create(String name) {
    if (name == null || name.isEmpty()) {
      System.err.println("Usage: hara new <project-name>");
      return;
    }

    File projectDir = new File(name);
    if (projectDir.exists()) {
      System.err.println("Directory " + name + " already exists.");
      return;
    }

    projectDir.mkdirs();
    new File(projectDir, SRC_DIR).mkdirs();

    String projectContent = "{:name \"" + name + "\"\n :dependencies []}";
    String mainContent = "(prn \"Hello, World from " + name + "!\")";

    try {
      Files.write(new File(projectDir, PROJECT_FILE).toPath(), projectContent.getBytes());
      Files.write(
          new File(new File(projectDir, SRC_DIR), MAIN_FILE).toPath(), mainContent.getBytes());
      System.out.println("Created new project: " + name);
    } catch (IOException e) {
      System.err.println("Failed to create project files: " + e.getMessage());
      e.printStackTrace();
    }
  }

  private static RT.Instance loadContext() {
    File projectFile = new File(PROJECT_FILE);
    if (!projectFile.exists()) {
      System.err.println(PROJECT_FILE + " not found. Are you in a hara project directory?");
      System.exit(1);
    }

    try {
      String content = new String(Files.readAllBytes(projectFile.toPath()));
      Object projectData = Parser.LispReader.readString(content, null);

      if (!(projectData instanceof IMapType)) {
        throw new Ex.Runtime("Invalid project.hara format. Expected a map.");
      }

      // Initialize Runtime
      var F = new Foundation();
      var rt = new RT.Instance<>(F, "ROOT");
      F.RTS.put(rt._key, rt);

      // Add src directory to classpath
      File srcDir = new File(SRC_DIR);
      if (srcDir.exists() && !NativeMode.enabled()) {
        java.net.URL srcUrl = srcDir.toURI().toURL();
        rt.pathAdd(srcUrl);
      }

      // Load Dependencies
      // Use ILookup to access map values
      // Fix: Use Keyword for lookup since keys are keywords like :dependencies
      Object depsObj = ((ILookup) projectData).lookup(Keyword.create("dependencies"), null);

      if (depsObj instanceof Iterable) {
        Iterable deps = (Iterable) depsObj;
        Iterator it = deps.iterator();
        if (NativeMode.enabled()) {
          // Empty dependency lists remain valid in native mode so bare projects can still run.
          if (it.hasNext()) {
            throw NativeMode.unsupported("dynamic Maven dependency loading from project.hara");
          }
        } else {
          while (it.hasNext()) {
            Object dep = it.next();
            if (dep instanceof String) {
              String coordinate = (String) dep;
              System.out.println("Loading dependency: " + coordinate);
              Maven.load.invoke(rt, coordinate);
            }
          }
        }
      }

      return rt;
    } catch (IOException e) {
      throw Ex.Sneaky(e);
    }
  }

  public static void run() {
    RT.Instance rt = loadContext();
    Path mainPath = Paths.get(SRC_DIR, MAIN_FILE);
    if (!Files.exists(mainPath)) {
      System.err.println("Entry file not found: " + mainPath);
      System.exit(1);
    }

    try {
      String mainScript = new String(Files.readAllBytes(mainPath));

      hara.kernel.base.Reader rdr =
          new hara.kernel.base.Reader(new java.io.StringReader(mainScript));

      // Use a unique object for EOF marker
      Object EOF = new Object();

      while (true) {
        // Call LispReader.read with eofIsError=false and our EOF marker
        Object form = Parser.LispReader.read(rdr, false, EOF, false, null);
        if (form == EOF) {
          break;
        }
        rt.eval(form);
      }

    } catch (IOException e) {
      throw Ex.Sneaky(e);
    }
  }

  public static void repl() throws IOException {
    NativeMode.requireDisabled("project REPL");
    RT.Instance rt = loadContext();

    Foundation F = (Foundation) rt.getRoot();
    var server = new Server(F, "PRIMARY", Foundation.DEFAULT_PORT, null);
    F.SERVERS.put(server._key, server);
    server.start();

    System.out.println("Starting REPL with project context...");
    Repl repl = new Repl(rt);
    repl.run();
  }

  public static void setup(String[] args) throws IOException {
    NativeMode.requireDisabled("project setup");
    if (args.length == 0) {
      System.err.println("Usage: hara setup <config-file> [--wait | --repl <runtime-name>]");
      return;
    }

    String configFile = args[0];
    boolean wait = false;
    String replRuntime = null;

    // Parse lifecycle args
    for (int i = 1; i < args.length; i++) {
      if ("--wait".equals(args[i])) {
        wait = true;
      } else if ("--repl".equals(args[i])) {
        if (i + 1 < args.length) {
          replRuntime = args[i + 1];
          i++;
        } else {
          System.err.println("Missing runtime name for --repl");
          return;
        }
      }
    }

    // 1. Load Root Context (to get Foundation and dependencies)
    RT.Instance rootRt = loadContext();
    Foundation F = (Foundation) rootRt.getRoot();

    // 2. Read Config File
    File config = new File(configFile);
    if (!config.exists()) {
      System.err.println("Config file not found: " + configFile);
      return;
    }

    String content = new String(Files.readAllBytes(config.toPath()));
    Object configData = Parser.LispReader.readString(content, null);

    if (!(configData instanceof IMapType)) {
      throw new Ex.Runtime("Invalid setup config format. Expected a map.");
    }

    // 3. Process Runtimes
    Object runtimesObj = ((ILookup) configData).lookup(Keyword.create("runtimes"), null);
    if (runtimesObj instanceof Iterable) {
      Iterable<java.util.Map.Entry> runtimesMap = (Iterable<java.util.Map.Entry>) runtimesObj;
      Iterator<java.util.Map.Entry> it = runtimesMap.iterator();

      while (it.hasNext()) {
        java.util.Map.Entry entry = it.next();
        Object keyObj = entry.getKey();
        String name;
        if (keyObj instanceof hara.lang.protocol.INamespaced) {
          name = ((hara.lang.protocol.INamespaced) keyObj).getName();
        } else {
          name = keyObj.toString();
          // Handle Keyword-like strings if any (though unlikely with Object key)
          if (name.startsWith(":")) {
            name = name.substring(1);
          }
        }

        Object rtConfig = entry.getValue();
        if (rtConfig instanceof ILookup) {
          Object srcObj = ((ILookup) rtConfig).lookup(Keyword.create("src"), null);
          if (srcObj instanceof String) {
            String srcPath = (String) srcObj;
            System.out.println("Creating runtime '" + name + "' loading " + srcPath + "...");

            // Create Runtime
            RT.Instance newRt = new RT.Instance<>(F, name);
            F.RTS.put(name, newRt);

            // Add src dir to path so it can find files relative to project root
            File srcDir = new File(SRC_DIR);
            if (srcDir.exists()) {
              newRt.pathAdd(srcDir.toURI().toURL());
            }

            // Load the source file
            Path path = Paths.get(srcPath);
            if (Files.exists(path)) {
              String script = new String(Files.readAllBytes(path));
              hara.kernel.base.Reader rdr =
                  new hara.kernel.base.Reader(new java.io.StringReader(script));
              Object EOF = new Object();
              while (true) {
                Object form = Parser.LispReader.read(rdr, false, EOF, false, null);
                if (form == EOF) break;
                newRt.eval(form);
              }
            } else {
              System.err.println("Warning: Source file " + srcPath + " not found.");
            }
          }
        }
      }
    }

    // 4. Handle Lifecycle
    var server = new Server(F, "PRIMARY", Foundation.DEFAULT_PORT, null);
    F.SERVERS.put(server._key, server);
    server.start();

    if (replRuntime != null) {
      RT.Instance rt = (RT.Instance) F.RTS.get(replRuntime);
      if (rt != null) {
        System.out.println("Attaching REPL to runtime: " + replRuntime);
        Repl repl = new Repl(rt);
        repl.run();
      } else {
        System.err.println("Runtime not found: " + replRuntime);
      }
    } else if (wait) {
      System.out.println("System running in wait mode. Press Ctrl+C to exit.");
      try {
        Thread.currentThread().join();
      } catch (InterruptedException e) {
        Thread.currentThread().interrupt();
      }
    }
  }
}
