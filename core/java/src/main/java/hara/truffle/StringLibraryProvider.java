package hara.truffle;

/** Native String substrate used by the source-owned Foundation string library. */
public final class StringLibraryProvider implements HaraLibraryProvider {
  @Override
  public String namespace() { return "std.native.String"; }

  @Override
  public int order() { return 20; }

  @Override
  public boolean eager() { return true; }

  @Override
  public void install(HaraContext context) {
    context.installNativeLibrary(namespace());
  }
}
