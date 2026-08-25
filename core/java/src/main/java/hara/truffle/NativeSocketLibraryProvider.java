package hara.truffle;

/** Lazy Java implementation of {@code std.native.Socket}. */
public final class NativeSocketLibraryProvider implements HaraLibraryProvider {
  @Override
  public String namespace() { return "std.native.Socket"; }

  @Override
  public int order() { return 20; }

  @Override
  public void install(HaraContext context) {
    context.installNativeLibrary(namespace());
  }
}
