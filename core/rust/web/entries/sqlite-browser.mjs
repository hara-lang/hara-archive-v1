import sqlite3InitModule from "@sqlite.org/sqlite-wasm";
import { createSqliteProvider } from "@hara-lang/db-sqlite";

const sqlite = createSqliteProvider(sqlite3InitModule);

export default (operation, args, context) =>
  sqlite.call("browser", operation, args, context);

export const close = () => sqlite.closeAll();
