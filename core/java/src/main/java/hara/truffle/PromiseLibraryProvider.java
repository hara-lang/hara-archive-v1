package hara.truffle;

/** Native Promise substrate used by the source-owned Foundation promise library. */
public final class PromiseLibraryProvider implements HaraLibraryProvider {
  @Override
  public String namespace() { return "std.native.Promise"; }

  @Override
  public int order() { return 20; }

  @Override
  public void install(HaraContext context) {
    context.installNativeLibrary(namespace());
  }
}
