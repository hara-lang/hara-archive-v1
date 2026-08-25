package hara.truffle;

/** Lazy Java implementation of {@code std.native.OS}. */
public final class NativeOsLibraryProvider implements HaraLibraryProvider {
  @Override public String namespace() { return "std.native.OS"; }
  @Override public int order() { return 20; }
  @Override public boolean eager() { return true; }
  @Override public void install(HaraContext context) {
    context.installNativeLibrary(namespace());
  }
}
