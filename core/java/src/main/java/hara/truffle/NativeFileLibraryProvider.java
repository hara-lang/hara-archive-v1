package hara.truffle;

/** Lazy Java implementation of {@code std.native.File}. */
public final class NativeFileLibraryProvider implements HaraLibraryProvider {
  @Override
  public String namespace() { return "std.native.File"; }

  @Override
  public int order() { return 20; }

  @Override
  public void install(HaraContext context) {
    context.installNativeLibrary(namespace());
  }
}
