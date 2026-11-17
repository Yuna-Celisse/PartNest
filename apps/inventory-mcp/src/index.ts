import { serveStdio } from "@modelcontextprotocol/server/stdio";
import { createInventoryServer } from "./server";
import { defaultDatabasePath } from "./inventory";

const args = process.argv.slice(2);
if (args.length && (args.length !== 2 || args[0] !== "--database" || !args[1])) {
  console.error("Usage: node dist/index.mjs [--database <partnest.db>]");
  process.exit(1);
}
const databasePath = args[1] ?? process.env.PARTNEST_DATABASE ?? defaultDatabasePath();
serveStdio(() => createInventoryServer(databasePath));
