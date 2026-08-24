package hara.truffle;

/** Installs the raw native Work surface; protocol dispatch comes from annotations. */
final class HaraNativeWork {
  private static final String NAMESPACE = "std.native.Work";

  private HaraNativeWork() {}

  static void install(HaraContext context) {
    context.defineNativeFunction(
        NAMESPACE,
        "default-host",
        arguments -> {
          requireArity("default-host", arguments, 0);
          return HaraWorkHost.instance();
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "current-run",
        arguments -> {
          requireArity("current-run", arguments, 0);
          HaraWorkHost.WorkContext current = HaraWorkHost.currentWorkContext();
          return current == null ? null : current.currentRun();
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "cancelled?",
        arguments -> requireCurrent("cancelled?", arguments, 0).cancelled(),
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "check-cancelled",
        arguments -> {
          requireCurrent("check-cancelled", arguments, 0).checkCancelled();
          return null;
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "deadline-nanos",
        arguments -> requireCurrent("deadline-nanos", arguments, 0).deadlineNanos(),
        null);
    context.defineNativeFunction(
        NAMESPACE,
        "emit",
        arguments -> {
          requireArity("emit", arguments, 2);
          return requireCurrent("emit", arguments, 2).emit(arguments[0], arguments[1]);
        },
        null);
    context.defineNativeFunction(
        NAMESPACE,
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
    context.defineNativeFunction(
        NAMESPACE,
        "on-close",
        arguments -> {
          requireArity("on-close", arguments, 1);
          Object function = arguments[0];
          Object wrapper =
              context.libraryFunction(
                  NAMESPACE + "/on-close-finalizer",
                  ignored ->
                      context.invokeCallable(
                          function,
                          new Object[] {
                            requireCurrent("on-close", new Object[0], 0).currentRun()
                          }));
          return requireCurrent("on-close", arguments, 1).onClose(wrapper);
        },
        null);
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
}
