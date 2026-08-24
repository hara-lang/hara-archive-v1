package hara.truffle;

import hara.lang.data.Keyword;
import hara.lang.protocol.ICoroutine;
import hara.lang.protocol.IDeref;
import com.oracle.truffle.api.bytecode.ContinuationResult;
import java.util.concurrent.SynchronousQueue;
import org.graalvm.nativeimage.ImageInfo;

/** Static Java implementation exported through std.native.Coroutine/*. */
public final class StdFoundationCoroutine {
  private StdFoundationCoroutine() {}

  private static final ThreadLocal<HaraCoroutine> CURRENT = new ThreadLocal<>();

  static final Keyword STATUS_SUSPENDED = Keyword.create("suspended");
  static final Keyword STATUS_RUNNING = Keyword.create("running");
  static final Keyword STATUS_DEAD = Keyword.create("dead");

  /** Thrown on the coroutine thread when a parked coroutine is closed. Unwinds the body. */
  static final class CoroutineClosed extends RuntimeException {
    CoroutineClosed() {
      super(null, null, false, false);
    }
  }

  /** One handoff across the resume/yield boundary. */
  static final class Transfer {
    final Object value;
    final Throwable error;

    Transfer(Object value, Throwable error) {
      this.value = value;
      this.error = error;
    }
  }

  /** A coroutine backed by a parked (virtual) thread. */
  static final class HaraCoroutine implements ICoroutine {
    /** Queue stand-in for nil: SynchronousQueue rejects null elements. */
    private static final Object NIL = new Object();

    private final HaraContext context;
    private final Object function;
    private final SynchronousQueue<Object> input = new SynchronousQueue<>();
    private final SynchronousQueue<Transfer> output = new SynchronousQueue<>();
    private volatile Keyword status = STATUS_SUSPENDED;
    private volatile Thread thread;

    HaraCoroutine(HaraContext context, Object function) {
      this.context = context;
      this.function = function;
    }

    @Override
    public Keyword status() {
      return status;
    }

    Object closeCoroutine() {
      Thread toInterrupt;
      synchronized (this) {
        if (status == STATUS_DEAD) return this;
        if (status == STATUS_RUNNING) {
          throw new HaraException("coroutine/close: cannot close a running coroutine");
        }
        status = STATUS_DEAD;
        toInterrupt = thread;
      }
      if (toInterrupt != null) toInterrupt.interrupt();
      return this;
    }

    @Override
    public void close() {
      closeCoroutine();
    }

    @Override
    public Object resume(Object... args) {
      synchronized (this) {
        if (status == STATUS_DEAD) {
          throw new HaraException("coroutine/resume: cannot resume a dead coroutine");
        }
        if (status == STATUS_RUNNING) {
          throw new HaraException("coroutine/resume: cannot resume a running coroutine");
        }
        status = STATUS_RUNNING;
      }
      if (thread == null) {
        start(args);
      } else {
        putInput(pack(args));
      }
      Transfer transfer = takeOutput();
      if (transfer.error != null) {
        if (transfer.error instanceof CoroutineClosed) {
          throw new HaraException("coroutine/resume: coroutine closed during resume");
        }
        if (transfer.error instanceof RuntimeException) throw (RuntimeException) transfer.error;
        if (transfer.error instanceof Error) throw (Error) transfer.error;
        throw new HaraException("coroutine failed: " + transfer.error);
      }
      return transfer.value;
    }

    private void start(Object[] args) {
      Runnable body = () -> runBody(args);
      Thread started =
          ImageInfo.inImageRuntimeCode()
              ? new Thread(body, "hara-coroutine")
              : Thread.ofVirtual().name("hara-coroutine").unstarted(body);
      thread = started;
      started.start();
    }

    Object doYield(Object value) {
      if (status == STATUS_DEAD) throw new CoroutineClosed();
      status = STATUS_SUSPENDED;
      putOutput(new Transfer(value, null));
      return takeInput();
    }

    private void runBody(Object[] args) {
      CURRENT.set(this);
      try {
        Object result = context.invokeInContext(() -> context.invokeCallable(function, args));
        while (result instanceof ContinuationResult continuation) {
          status = STATUS_SUSPENDED;
          putOutput(new Transfer(continuation.getResult(), null));
          Object resumeValue = takeInput();
          result =
              context.invokeInContext(() -> continuation.continueWith(resumeValue));
        }
        status = STATUS_DEAD;
        putOutput(new Transfer(result, null));
      } catch (CoroutineClosed closed) {
        // Closed while parked: the unwinding already ran the body's finally clauses.
        // Best-effort release of a resumer blocked in takeOutput(); a no-op when none waits.
        output.offer(new Transfer(null, closed));
        Thread.currentThread().interrupt();
      } catch (Throwable error) {
        status = STATUS_DEAD;
        putOutput(new Transfer(null, error));
      } finally {
        CURRENT.remove();
      }
    }

    private void putInput(Object value) {
      try {
        input.put(value == null ? NIL : value);
      } catch (InterruptedException error) {
        Thread.currentThread().interrupt();
        status = STATUS_DEAD;
        Thread parked = thread;
        if (parked != null) parked.interrupt();
        throw new HaraException("coroutine/resume: interrupted while resuming");
      }
    }

    private void putOutput(Transfer transfer) {
      try {
        output.put(transfer);
      } catch (InterruptedException error) {
        Thread.currentThread().interrupt();
        throw new CoroutineClosed();
      }
    }

    private Object takeInput() {
      try {
        Object value = input.take();
        return value == NIL ? null : value;
      } catch (InterruptedException error) {
        Thread.currentThread().interrupt();
        throw new CoroutineClosed();
      }
    }

    private Transfer takeOutput() {
      try {
        return output.take();
      } catch (InterruptedException error) {
        Thread.currentThread().interrupt();
        status = STATUS_DEAD;
        Thread parked = thread;
        if (parked != null) parked.interrupt();
        throw new HaraException("coroutine/resume: interrupted while waiting for the coroutine");
      }
    }

    @Override
    public String toString() {
      return "#<coroutine " + status + ">";
    }
  }

  private static void requireArity(String name, Object[] values, int expected) {
    if (values.length != expected) {
      throw new HaraException(
          name + " expects " + expected + " argument(s), got " + values.length);
    }
  }

  private static HaraCoroutine requireCoroutine(String name, Object value) {
    Object unwrapped = HaraBox.unwrap(value);
    if (!(unwrapped instanceof HaraCoroutine)) {
      throw new HaraException(name + " expects a coroutine");
    }
    return (HaraCoroutine) unwrapped;
  }

  private static Object pack(Object[] values) {
    if (values.length == 0) return null;
    if (values.length == 1) return values[0];
    return hara.lang.data.Vector.Standard.from(null, values.clone());
  }

  static void install(HaraContext context, String namespace) {
    HaraNativeLibrary.function(context, namespace, "create", StdFoundationCoroutine::create,
        "Creates a coroutine wrapping f. The body does not start until the first resume.", "[f]");
    HaraNativeLibrary.function(context, namespace, "coroutine?",
        StdFoundationCoroutine::coroutinePredicate, "Returns true when value is a coroutine.",
        "[value]");
    HaraNativeLibrary.function(context, namespace, "status", StdFoundationCoroutine::status,
        "Returns the coroutine status: :suspended, :running, or :dead.", "[co]");
    HaraNativeLibrary.function(context, namespace, "close", StdFoundationCoroutine::close,
        "Closes a suspended coroutine: marks it dead and unwinds it, running finally clauses."
            + " No-op on a dead coroutine. Throws on a running coroutine.",
        "[co]");
    HaraNativeLibrary.function(context, namespace, "resume", StdFoundationCoroutine::resume,
        "Starts or continues a coroutine. First resume passes args to the body; later resumes"
            + " deliver args as the yield return. Returns the yielded or final value.",
        "[co & args]");
    HaraNativeLibrary.function(context, namespace, "yield", StdFoundationCoroutine::yield,
        "Suspends the current coroutine and yields one value. The next resume's arguments"
            + " become this expression's return. Throws outside a coroutine.",
        "[value]");
    HaraNativeLibrary.function(context, namespace, "await", StdFoundationCoroutine::await,
        "Blocks the current (coroutine) thread until the promise settles and returns its"
            + " value. Rethrows on rejection. Outside a coroutine, behaves as a plain deref.",
        "[p]");
  }

  public static Object create(HaraContext context, Object[] values) {
    requireArity("coroutine/create", values, 1);
    Object f = HaraBox.unwrap(values[0]);
    if (!(f instanceof HaraFunction)
        && !(f instanceof HaraMultiFunction)
        && !(f instanceof HbcMachine.HbcClosure)
        && !(f instanceof HbcMachine.HbcMultiArity)
        && !(f instanceof hara.lang.protocol.IFn)) {
      throw new HaraException("coroutine/create expects a function");
    }
    return new HaraCoroutine(context, f);
  }

  public static Object coroutinePredicate(HaraContext context, Object[] values) {
    requireArity("coroutine/coroutine?", values, 1);
    return HaraBox.unwrap(values[0]) instanceof HaraCoroutine;
  }

  public static Object status(HaraContext context, Object[] values) {
    requireArity("coroutine/status", values, 1);
    return requireCoroutine("coroutine/status", values[0]).status();
  }

  public static Object close(HaraContext context, Object[] values) {
    requireArity("coroutine/close", values, 1);
    return requireCoroutine("coroutine/close", values[0]).closeCoroutine();
  }

  public static Object resume(HaraContext context, Object[] values) {
    if (values.length == 0) {
      throw new HaraException("coroutine/resume expects a coroutine");
    }
    HaraCoroutine coroutine = requireCoroutine("coroutine/resume", values[0]);
    Object[] args = new Object[values.length - 1];
    System.arraycopy(values, 1, args, 0, args.length);
    return coroutine.resume(args);
  }

  public static Object yield(HaraContext context, Object[] values) {
    requireArity("coroutine/yield", values, 1);
    HaraCoroutine coroutine = CURRENT.get();
    if (coroutine == null) {
      throw new HaraException("coroutine/yield: cannot yield outside a coroutine");
    }
    return coroutine.doYield(values[0]);
  }

  /** Checks the guest continuation boundary used by native HBC yield operations. */
  public static void requireYieldContext() {
    if (CURRENT.get() == null) {
      throw new HaraException("coroutine/yield: cannot yield outside a coroutine");
    }
  }

  public static Object await(HaraContext context, Object[] values) {
    requireArity("coroutine/await", values, 1);
    Object value = HaraBox.unwrap(values[0]);
    if (!(value instanceof IDeref)) {
      throw new HaraException("coroutine/await expects a derefable (e.g. a promise)");
    }
    return ((IDeref<?>) value).deref();
  }
}
