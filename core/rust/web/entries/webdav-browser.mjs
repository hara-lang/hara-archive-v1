import { serveBrowserProvider } from "@hara-lang/hta/provider/browser";
import { createWebdavProvider } from "@hara-lang/fs-webdav";

const webdav = createWebdavProvider();

serveBrowserProvider(
  (operation, args, context) => webdav.call("browser", operation, args, context),
  {
    errorCode: "file/io",
    close: () => webdav.closeAll()
  }
);
