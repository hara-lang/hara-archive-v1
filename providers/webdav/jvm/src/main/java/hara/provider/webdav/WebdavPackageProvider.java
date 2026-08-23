package hara.provider.webdav;

import hara.truffle.JvmPackageProvider;
import hara.truffle.WebdavFilesystem;
import java.util.Set;

/** Host-only JVM package entry point for the WebDAV filesystem provider. */
public final class WebdavPackageProvider implements JvmPackageProvider {
  public static final String IDENTITY = "hara:hara/filesystem-webdav";

  @Override
  public String identity() {
    return IDENTITY;
  }

  @Override
  public Set<String> capabilities() {
    return Set.of();
  }

  @Override
  public void register(Registration registration) {
    registration.filesystem(new WebdavFilesystem.Factory());
  }
}
