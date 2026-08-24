package hara.truffle;

import hara.lang.data.Symbol;
import hara.lang.protocol.IWork;
import hara.lang.protocol.IWorkExecutor;
import hara.lang.protocol.IWorkHost;
import hara.lang.protocol.IWorkRef;
import hara.lang.protocol.IWorkRun;
import hara.lang.protocol.IWorkStore;

/** Installs the native work protocol identities and current-run helpers. */
public final class WorkProtocolLibraryProvider implements HaraLibraryProvider {
  @Override
  public String namespace() {
    return "work.native";
  }

  @Override
  public int order() {
    return 1000;
  }

  @Override
  public boolean eager() {
    return true;
  }

  @Override
  public void install(HaraContext context) {
    requireProtocol(context, "IWork");
    requireProtocol(context, "IWorkExecutor");
    requireProtocol(context, "IWorkStore");
    requireProtocol(context, "IWorkRef");
    requireProtocol(context, "IWorkHost");
    requireProtocol(context, "IWorkRun");
    context.defineLibraryValue(namespace(), "default-host", HaraWorkHost.instance(), null);
    context.defineLibraryFunction(
        namespace(),
        "current-run",
        arguments -> {
          requireArity("current-run", arguments, 0);
          HaraWorkHost.WorkContext current = HaraWorkHost.currentWorkContext();
          return current == null ? null : current.currentRun();
        },
        null);
    context.defineLibraryFunction(
        namespace(),
        "cancelled?",
        arguments -> requireCurrent("cancelled?", arguments, 0).cancelled(),
        null);
    context.defineLibraryFunction(
        namespace(),
        "check-cancelled",
        arguments -> {
          requireCurrent("check-cancelled", arguments, 0).checkCancelled();
          return null;
        },
        null);
    context.defineLibraryFunction(
        namespace(),
        "deadline-nanos",
        arguments -> requireCurrent("deadline-nanos", arguments, 0).deadlineNanos(),
        null);
    context.defineLibraryFunction(
        namespace(),
        "emit",
        arguments -> {
          requireArity("emit", arguments, 2);
          return requireCurrent("emit", arguments, 2).emit(arguments[0], arguments[1]);
        },
        null);
    context.defineLibraryFunction(
        namespace(),
        "submit-child",
        arguments -> {
          if (arguments.length == 2) {
            return requireCurrent("submit-child", arguments, 2)
                .submitChild(arguments[0], arguments[1], null);
          }
          if (arguments.length == 3) {
            return requireCurrent("submit-child", arguments, 3)
                .submitChild(arguments[0], arguments[1], arguments[2]);
          }
          throw new HaraException("submit-child expects 2 or 3 arguments");
        },
        null);
    context.defineLibraryFunction(
        namespace(),
        "on-close",
        arguments -> {
          requireArity("on-close", arguments, 1);
          Object function = arguments[0];
          Object wrapper =
              context.libraryFunction(
                  "work.native/on-close-finalizer",
                  ignored ->
                      context.invokeCallable(
                          function,
                          new Object[] {requireCurrent("on-close", new Object[0], 0).currentRun()}));
          return requireCurrent("on-close", arguments, 1).onClose(wrapper);
        },
        null);
  }

  static void installWork(HaraProtocol protocol) {
    protocol.extend(
        IWork.class,
        "work-spec",
        (receiver, arguments) -> ((IWork) receiver).workSpec());
  }

  static void installWorkExecutor(HaraProtocol protocol) {
    protocol.extend(
        IWorkExecutor.class,
        "work-execute",
        (receiver, arguments) -> ((IWorkExecutor) receiver).workExecute(arguments[0]));
  }

  static void installWorkStore(HaraProtocol protocol) {
    protocol.extend(
        IWorkStore.class,
        "work-query",
        (receiver, arguments) -> ((IWorkStore) receiver).workQuery(arguments[0]));
    protocol.extend(
        IWorkStore.class,
        "work-transact",
        (receiver, arguments) -> ((IWorkStore) receiver).workTransact(arguments[0]));
  }

  static void installWorkRef(HaraProtocol protocol) {
    protocol.extend(
        IWorkRef.class,
        "work-id",
        (receiver, arguments) -> ((IWorkRef) receiver).workId());
  }

  static void installWorkHost(HaraProtocol protocol) {
    protocol.extend(
        IWorkHost.class,
        "work-submit",
        (receiver, arguments) ->
            ((IWorkHost) receiver).workSubmit(arguments[0], arguments[1], arguments[2]));
    protocol.extend(
        IWorkHost.class,
        "work-resolve",
        (receiver, arguments) -> ((IWorkHost) receiver).workResolve(arguments[0]));
  }

  static void installWorkRun(HaraProtocol protocol) {
    protocol.extend(
        IWorkRun.class,
        "work-status",
        (receiver, arguments) -> ((IWorkRun) receiver).workStatus());
    protocol.extend(
        IWorkRun.class,
        "work-result",
        (receiver, arguments) -> ((IWorkRun) receiver).workResult());
    protocol.extend(
        IWorkRun.class,
        "work-events",
        (receiver, arguments) -> ((IWorkRun) receiver).workEvents(arguments[0]));
    protocol.extend(
        IWorkRun.class,
        "work-cancel",
        (receiver, arguments) -> ((IWorkRun) receiver).workCancel(arguments[0]));
  }

  private static HaraWorkHost.WorkContext requireCurrent(
      String name, Object[] arguments, int arity) {
    requireArity(name, arguments, arity);
    HaraWorkHost.WorkContext workContext = HaraWorkHost.currentWorkContext();
    if (workContext == null) {
      throw new HaraException(name + " requires an active native work context");
    }
    return workContext;
  }

  private static void requireArity(String name, Object[] arguments, int arity) {
    if (arguments.length != arity) {
      throw new HaraException(name + " expects " + arity + " arguments");
    }
  }

  private static void requireProtocol(HaraContext context, String name) {
    HaraVar variable = context.resolve(Symbol.create("std.foundation", name));
    if (variable == null || !(variable.get() instanceof HaraProtocol)) {
      throw new HaraException("Native work protocol is unavailable: " + name);
    }
  }

}
