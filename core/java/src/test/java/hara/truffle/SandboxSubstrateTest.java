package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertThrows;

import java.util.List;
import java.nio.file.Files;
import java.nio.file.Path;
import org.junit.Test;

public class SandboxSubstrateTest {
  private static Object eval(
      SessionKernel kernel, SandboxModel.SandboxId sandbox, String source) {
    return kernel.sandboxEval(sandbox, source).await();
  }

  private static Object call(
      SessionKernel kernel,
      SandboxModel.SandboxId sandbox,
      String callable,
      List<Object> arguments) {
    return kernel.sandboxCall(sandbox, callable, arguments).await();
  }

  @Test
  public void nativeSandboxSurfaceUsesTheOwningKernel() {
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      kernel.root().eval("(require 'std.lib.kernel)");
      long id =
          kernel
              .root()
              .eval(
                  "(deref (std.lib.kernel/sandbox-open {:protocol \"hara.sandbox/0-alpha\" "
                      + ":provider :in-process :runtime \"hara.standard/0-alpha\" "
                      + ":entry-namespace 'user :bundles [] :mount nil :provider-options {} "
                      + ":limits {:source-bytes 65536 :result-bytes 1048576 "
                      + ":output-bytes 1048576 :evaluation-ms 5000 :memory-bytes 67108864 "
                      + ":active-evaluations 1}}))")
              .asLong();
      assertEquals(
          42L,
          kernel.root().eval("(deref (std.lib.kernel/sandbox-eval " + id + " \"(+ 40 2)\"))").asLong());
      assertEquals(
          6L,
          kernel
              .root()
              .eval("(deref (std.lib.kernel/sandbox-call " + id + " 'std.foundation/+ [1 2 3]))")
              .asLong());
      assertFalse(
          kernel
              .root()
              .eval("(:sandbox/secure (std.lib.kernel/sandbox-status " + id + "))")
              .asBoolean());
      kernel.root().eval("(deref (std.lib.kernel/sandbox-close " + id + "))");
      assertThrows(
          RuntimeException.class,
          () -> kernel.root().eval("(std.lib.kernel/sandbox-status " + id + ")"));
      assertEquals(
          ":sandbox/invalid-spec",
          kernel
              .root()
              .eval(
                  "(try (deref (std.lib.kernel/sandbox-open {:unknown true})) "
                      + "(catch Throwable error (:ex/code (ex-data error))))")
              .toString());
    }
  }

  @Test
  public void bundlesAndMountsAreResolvedAndReleasedByTheKernel() throws Exception {
    Path root = Files.createTempDirectory("hara-sandbox-mount-");
    try (SessionKernel kernel = new SessionKernel(true, false)) {
      String digest =
          "sha256:039058c6f2c0cb492c533b0a4d14ef77cc0f78abccced5287d84a1a2011cfb81";
      kernel.registerBundle(digest, new byte[] {1, 2, 3});
      SessionModel.SessionMountId mount = kernel.createFilesystem(root);
      SandboxModel.SandboxSpec spec =
          new SandboxModel.SandboxSpec(
              SandboxModel.SPEC_PROTOCOL,
              "in-process",
              "hara.standard/0-alpha",
              "user",
              List.of(new SandboxModel.BundleReference(digest, "halc")),
              mount,
              HaraPersistentValues.normalize(java.util.Map.of()),
              SandboxModel.SandboxLimits.defaults());
      SandboxModel.SandboxId sandbox = kernel.openSandbox(spec);
      assertEquals(1, kernel.filesystemInfo(mount).attachments());
      assertThrows(IllegalArgumentException.class, () -> kernel.closeFilesystem(mount));
      kernel.closeSandbox(sandbox);
      assertEquals(0, kernel.filesystemInfo(mount).attachments());
      kernel.closeFilesystem(mount);

      SandboxModel.SandboxSpec missing =
          new SandboxModel.SandboxSpec(
              SandboxModel.SPEC_PROTOCOL,
              "in-process",
              "hara.standard/0-alpha",
              "user",
              List.of(
                  new SandboxModel.BundleReference(
                      "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                      "halc")),
              null,
              HaraPersistentValues.normalize(java.util.Map.of()),
              SandboxModel.SandboxLimits.defaults());
      SandboxModel.SandboxException error =
          assertThrows(SandboxModel.SandboxException.class, () -> kernel.openSandbox(missing));
      assertEquals(SandboxModel.ErrorCode.BUNDLE_NOT_FOUND, error.code());

      String mismatched =
          "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
      kernel.registerBundle(mismatched, new byte[] {9});
      SandboxModel.SandboxSpec mismatch =
          new SandboxModel.SandboxSpec(
              SandboxModel.SPEC_PROTOCOL,
              "in-process",
              "hara.standard/0-alpha",
              "user",
              List.of(new SandboxModel.BundleReference(mismatched, "halc")),
              null,
              HaraPersistentValues.normalize(java.util.Map.of()),
              SandboxModel.SandboxLimits.defaults());
      assertEquals(
          SandboxModel.ErrorCode.BUNDLE_DIGEST_MISMATCH,
          assertThrows(SandboxModel.SandboxException.class, () -> kernel.openSandbox(mismatch))
              .code());
    } finally {
      Files.deleteIfExists(root);
    }
  }

  @Test
  public void inProcessLifecycleIsPrivateAndExplicitlyNonSecure() {
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SandboxProvider provider = InProcessSandboxProvider.INSTANCE;
      assertFalse(provider.secure());
      kernel.registerSandboxProvider(provider);
      int sessionsBefore = kernel.size();

      SandboxModel.SandboxId sandbox = kernel.openSandbox(SandboxModel.SandboxSpec.inProcess());
      assertEquals(sessionsBefore, kernel.size());
      assertEquals(41L, eval(kernel, sandbox, "(def answer 41) answer"));
      assertEquals(42L, call(kernel, sandbox, "std.foundation/+", List.of(41L, 1L)));
      String inertSource = "(do (def injected 99) :executed)";
      assertEquals(
          inertSource,
          call(kernel, sandbox, "std.foundation/identity", List.of(inertSource)));
      assertEquals(SandboxModel.SandboxState.OPEN, kernel.sandboxStatus(sandbox).state());
      assertFalse(kernel.cancelSandbox(sandbox));
      assertEquals(SandboxModel.SandboxState.OPEN, kernel.sandboxStatus(sandbox).state());

      kernel.closeSandbox(sandbox);
      SandboxModel.SandboxException error =
          assertThrows(SandboxModel.SandboxException.class, () -> kernel.sandboxStatus(sandbox));
      assertEquals(SandboxModel.ErrorCode.NOT_FOUND, error.code());

      SandboxModel.SandboxId injectionProbe =
          kernel.openSandbox(SandboxModel.SandboxSpec.inProcess());
      assertThrows(
          SandboxModel.SandboxException.class,
          () -> eval(kernel, injectionProbe, "injected"));
      kernel.closeSandbox(injectionProbe);
    }
  }

  @Test
  public void specValidationAndRuntimeIsolationAreEnforced() {
    SandboxModel.SandboxException invalid =
        assertThrows(
            SandboxModel.SandboxException.class,
            () -> new SandboxModel.SandboxLimits(1, 1, 1, 1, 1, 2));
    assertEquals(SandboxModel.ErrorCode.INVALID_SPEC, invalid.code());

    try (SessionKernel kernel = new SessionKernel(false, false)) {
      kernel.registerSandboxProvider(InProcessSandboxProvider.INSTANCE);
      kernel.root().eval("(def parent-secret 42)");
      SandboxModel.SandboxId parentProbe =
          kernel.openSandbox(SandboxModel.SandboxSpec.inProcess());
      SandboxModel.SandboxException error =
          assertThrows(
              SandboxModel.SandboxException.class,
              () -> eval(kernel, parentProbe, "parent-secret"));
      assertEquals(SandboxModel.ErrorCode.EVALUATION_FAILED, error.code());
      kernel.closeSandbox(parentProbe);
      SandboxModel.SandboxId sandbox = kernel.openSandbox(SandboxModel.SandboxSpec.inProcess());
      assertEquals(
          true,
          eval(
              kernel,
              sandbox,
              "(every? identity [(nil? (resolve 'Runtime)) (nil? (resolve 'Kernel)) "
                  + "(nil? (resolve 'Sandbox)) (nil? (resolve 'Crypto)) "
                  + "(nil? (resolve 'File)) (nil? (resolve 'Socket)) "
                  + "(nil? (resolve 'Process)) (nil? (resolve 'OS)) "
                  + "(nil? (resolve 'Package)) (nil? (resolve 'Host)) "
                  + "(nil? (resolve 'Runtime/resolve)) "
                  + "(nil? (resolve 'std.native.Runtime/current)) "
                  + "(nil? (resolve 'std.native.Runtime/resolve)) "
                  + "(nil? (resolve 'Host/call)) (nil? (resolve 'File/read)) "
                  + "(nil? (Base/resolve 'std.native.Runtime/resolve)) "
                  + "(nil? (resolve 'std.native.Kernel))])"));
      assertEquals(null, eval(kernel, sandbox, "(ns-find 'std.native.Kernel)"));
      assertEquals(false, eval(kernel, sandbox, "(ns-loaded? 'std.native.Runtime)"));
      assertEquals(
          hara.lang.data.Keyword.create("unknown"),
          eval(kernel, sandbox, "(ns-state 'std.native.Package)"));
      assertEquals(
          6L,
          eval(
              kernel,
              sandbox, "(do (defn sandbox-sum [xs] (reduce + 0 xs)) (sandbox-sum (map inc [0 1 2])))"));
      assertThrows(
          SandboxModel.SandboxException.class,
          () -> eval(kernel, sandbox, "(ns-publics 'std.native.File)"));
      kernel.closeSandbox(sandbox);
    }
  }

  @Test
  public void evaluationsAreBusyCancellableTimedAndTerminal() {
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      kernel.registerSandboxProvider(InProcessSandboxProvider.INSTANCE);
      SandboxModel.SandboxId cancellable =
          kernel.openSandbox(SandboxModel.SandboxSpec.inProcess());
      SandboxProvider.Pending<Object> pending =
          kernel.sandboxEval(cancellable, "(loop [] (recur))");
      SandboxModel.SandboxException busy =
          assertThrows(
              SandboxModel.SandboxException.class,
              () -> kernel.sandboxEval(cancellable, "42"));
      assertEquals(SandboxModel.ErrorCode.BUSY, busy.code());
      org.junit.Assert.assertTrue(kernel.cancelSandbox(cancellable));
      SandboxModel.SandboxException cancelled =
          assertThrows(SandboxModel.SandboxException.class, pending::await);
      assertEquals(SandboxModel.ErrorCode.CANCELLED, cancelled.code());
      assertEquals(
          SandboxModel.SandboxState.CANCELLED, kernel.sandboxStatus(cancellable).state());
      assertFalse(kernel.cancelSandbox(cancellable));
      SandboxModel.SandboxException terminal =
          assertThrows(
              SandboxModel.SandboxException.class,
              () -> kernel.sandboxEval(cancellable, "42"));
      assertEquals(SandboxModel.ErrorCode.CLOSED, terminal.code());
      kernel.closeSandbox(cancellable);

      SandboxModel.SandboxSpec timeoutSpec =
          new SandboxModel.SandboxSpec(
              SandboxModel.SPEC_PROTOCOL,
              "in-process",
              "hara.standard/0-alpha",
              "user",
              new SandboxModel.SandboxLimits(
                  64 * 1024, 1024 * 1024, 1024 * 1024, 5, 64L * 1024 * 1024, 1));
      SandboxModel.SandboxId timedSandbox = kernel.openSandbox(timeoutSpec);
      SandboxProvider.Pending<Object> timed =
          kernel.sandboxEval(timedSandbox, "(loop [] (recur))");
      SandboxModel.SandboxException timeout =
          assertThrows(SandboxModel.SandboxException.class, timed::await);
      assertEquals(SandboxModel.ErrorCode.TIMEOUT, timeout.code());
      assertEquals(
          SandboxModel.SandboxState.FAILED, kernel.sandboxStatus(timedSandbox).state());
      kernel.closeSandbox(timedSandbox);

      SandboxModel.SandboxId closingSandbox =
          kernel.openSandbox(SandboxModel.SandboxSpec.inProcess());
      SandboxProvider.Pending<Object> closing =
          kernel.sandboxEval(closingSandbox, "(loop [] (recur))");
      kernel.closeSandbox(closingSandbox);
      SandboxModel.SandboxException closeCancellation =
          assertThrows(SandboxModel.SandboxException.class, closing::await);
      assertEquals(SandboxModel.ErrorCode.CANCELLED, closeCancellation.code());
      SandboxModel.SandboxId closed = closingSandbox;
      SandboxModel.SandboxException missing =
          assertThrows(SandboxModel.SandboxException.class, () -> kernel.sandboxStatus(closed));
      assertEquals(SandboxModel.ErrorCode.NOT_FOUND, missing.code());

      SandboxModel.SandboxSpec smallResult =
          new SandboxModel.SandboxSpec(
              SandboxModel.SPEC_PROTOCOL,
              "in-process",
              "hara.standard/0-alpha",
              "user",
              new SandboxModel.SandboxLimits(64 * 1024, 2, 1024, 5_000, 1024 * 1024, 1));
      SandboxModel.SandboxId overflowing = kernel.openSandbox(smallResult);
      assertEquals(
          SandboxModel.ErrorCode.LIMIT_EXCEEDED,
          assertThrows(
                  SandboxModel.SandboxException.class,
                  () -> kernel.sandboxEval(overflowing, "\"abcd\"").await())
              .code());
      kernel.closeSandbox(overflowing);

      SandboxModel.SandboxId liveValue =
          kernel.openSandbox(SandboxModel.SandboxSpec.inProcess());
      assertEquals(
          SandboxModel.ErrorCode.RESULT_NOT_TRANSFERABLE,
          assertThrows(
                  SandboxModel.SandboxException.class,
                  () -> kernel.sandboxEval(liveValue, "(fn [] 1)").await())
              .code());
      kernel.closeSandbox(liveValue);
    }
  }
}
