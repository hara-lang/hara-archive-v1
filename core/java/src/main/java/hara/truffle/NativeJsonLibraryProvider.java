package hara.truffle;

/** Eager implementation of {@code std.native.Json}. */
public final class NativeJsonLibraryProvider implements HaraLibraryProvider {
  @Override public String namespace() { return "std.native.Json"; }

  @Override public int order() { return 10; }

  @Override public boolean eager() { return true; }

  @Override public void install(HaraContext context) {
    context.installNativeLibrary(namespace());
  }
}
