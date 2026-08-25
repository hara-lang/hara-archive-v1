import { PGlite } from "@electric-sql/pglite";
import { createPgliteProvider } from "@hara-lang/db-pglite";

const pglite = createPgliteProvider(PGlite);

export default (operation, args, context) =>
  pglite.call("browser", operation, args, context);

export const close = () => pglite.closeAll();
