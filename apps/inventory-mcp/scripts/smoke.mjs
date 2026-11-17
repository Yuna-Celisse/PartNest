import { Client } from "@modelcontextprotocol/client";
import { StdioClientTransport } from "@modelcontextprotocol/client/stdio";
import { DatabaseSync } from "node:sqlite";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";
import { homedir } from "node:os";
import { join } from "node:path";

const base = process.platform === "win32" ? process.env.APPDATA : process.platform === "darwin" ? join(homedir(), "Library/Application Support") : process.env.XDG_DATA_HOME ?? join(homedir(), ".local/share");
const database = process.argv[2] ?? process.env.PARTNEST_DATABASE ?? join(base, "com.partnest.desktop/partnest.db");
const snapshot = () => {
  const db = new DatabaseSync(database, { readOnly: true });
  try { return db.prepare("SELECT * FROM parts ORDER BY id").all(); } finally { db.close(); }
};
const hash = (rows) => createHash("sha256").update(JSON.stringify(rows)).digest("hex");
const before = snapshot();
const stocked = before.filter((row) => row.deleted_at === null && row.quantity > 0);
const sample = stocked.find((row) => row.lcsc_code);
assert(sample, "No stocked part with LCSC code available for smoke test");
const client = new Client({ name: "partnest-live-smoke", version: "0.1.0" });
try {
  await client.connect(new StdioClientTransport({ command: process.execPath, args: [fileURLToPath(new URL("../dist/index.mjs", import.meta.url)), "--database", database], stderr: "pipe" }));
  const { tools } = await client.listTools();
  assert.equal(tools.length, 4);
  const call = async (name, args) => {
    const result = await client.callTool({ name, arguments: args });
    assert(!result.isError, JSON.stringify(result.content));
    return result.structuredContent;
  };
  const search = await call("search_inventory", { limit: 200 });
  assert.equal(search.total, stocked.length);
  const detail = await call("get_inventory_part", { part_id: sample.id });
  assert.equal(detail.part.quantity, sample.quantity);
  const suggestion = await call("suggest_inventory_parts", { lcsc_code: sample.lcsc_code, quantity: 1 });
  assert.equal(suggestion.match_kind, "exact-lcsc");
  const bom = await call("check_bom_stock", { lines: [{ key: "first", lcsc_code: sample.lcsc_code, quantity: sample.quantity }, { key: "second", lcsc_code: sample.lcsc_code, quantity: sample.quantity }] });
  assert.equal(bom.exact_identity_covered, sample.quantity);
  assert.equal(bom.uncovered_quantity, sample.quantity);
  assert.equal(hash(snapshot()), hash(before), "Inventory changed during read-only smoke test");
  console.log(JSON.stringify({ status: "passed", tools: tools.map((tool) => tool.name), stocked_parts: search.total, schema_version: search.schema_version, inventory_unchanged: true }, null, 2));
} finally { await client.close(); }
