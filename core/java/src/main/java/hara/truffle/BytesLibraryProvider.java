package hara.truffle;

import hara.lang.declaration.HaraNativeBinding;

/** Native Bytes substrate used by the source-owned Foundation bytes library. */
@HaraNativeBinding(namespace = "std.native", name = "Bytes")
public final class BytesLibraryProvider implements HaraLibraryProvider {
  @Override
  public String namespace() { return "std.native.Bytes"; }

  @Override
  public int order() { return 20; }

  @Override
  public void install(HaraContext context) {
    context.installBytesLibrary();
  }
}
