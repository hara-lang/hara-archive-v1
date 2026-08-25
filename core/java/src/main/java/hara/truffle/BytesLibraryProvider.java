package hara.truffle;

/** Native Bytes substrate used by the source-owned Foundation bytes library. */
public final class BytesLibraryProvider implements HaraLibraryProvider {
  @Override
  public String namespace() { return "std.native.Bytes"; }

  @Override
  public int order() { return 20; }

  @Override
  public void install(HaraContext context) {
    context.installNativeLibrary(namespace());
  }
}
