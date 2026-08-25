import { createWebdavProvider } from "@hara-lang/fs-webdav";

const webdav = createWebdavProvider();

export default (operation, args, context) =>
  webdav.call("browser", operation, args, context);

export const close = () => webdav.closeAll();
