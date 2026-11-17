import { createRequire } from "node:module";
const { DatabaseSync } = createRequire(import.meta.url)("node:sqlite") as typeof import("node:sqlite");
import { mkdtempSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, expect, it } from "vitest";
import { InventoryService } from "../src/inventory";
import { Client } from "@modelcontextprotocol/client";
import { StdioClientTransport } from "@modelcontextprotocol/client/stdio";

const cleanup: (() => void)[] = [];
afterEach(() => { for (const dispose of cleanup.splice(0).reverse()) dispose(); });
function fixture() {
  const directory = mkdtempSync(join(tmpdir(), "partnest-mcp-"));
  const path = join(directory, "inventory.db");
  cleanup.push(() => rmSync(directory, { recursive: true, force: true }));
  const db = new DatabaseSync(path);
  cleanup.push(() => db.close());
  db.exec(`PRAGMA journal_mode=WAL;
    CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY); INSERT INTO schema_migrations VALUES(16);
    CREATE TABLE boxes(id INTEGER PRIMARY KEY,name TEXT); INSERT INTO boxes VALUES(1,'Drawer');
    CREATE TABLE parts(id TEXT PRIMARY KEY,name TEXT,category TEXT,package TEXT,manufacturer TEXT,mpn TEXT,lcsc_code TEXT,quantity INTEGER,box_id INTEGER,slot TEXT,note TEXT,version INTEGER,updated_at TEXT,deleted_at TEXT);
    INSERT INTO parts VALUES('r','10kΩ R0402','Resistors','R0402','Maker','R-10K','C1',5,1,'A1','',1,'2026-11-03',NULL);
    INSERT INTO parts VALUES('c','100nF C0402','Capacitors','0402','Maker','C-100','C2',30,1,'A2','',1,'2026-11-03',NULL);
    INSERT INTO parts VALUES('zero','10kΩ R0402','Resistors','0402','Maker','ZERO','C3',0,NULL,NULL,'',1,'2026-11-03',NULL);
    INSERT INTO parts VALUES('archived','10kΩ R0402','Resistors','0402','Maker','R-10K','C4',100,1,'A3','',1,'2026-11-03','2026-11-03');`);
  return { path, db, service: new InventoryService(path) };
}

it("reads live WAL data, normalizes passive searches and excludes archived/empty stock", () => {
  const { db, service } = fixture();
  expect(service.search({ limit: 20, offset: 0 }).total).toBe(2);
  expect(service.search({ kind: "capacitor", value: "0.1uF", package: "Capacitor_SMD:C_0402_1005Metric", limit: 20, offset: 0 }).parts.map((p) => p.id)).toEqual(["c"]);
  db.exec("UPDATE parts SET quantity=8,version=2 WHERE id='r'");
  expect(service.get("r").part).toMatchObject({ quantity: 8, version: 2, box_name: "Drawer" });
  expect(() => service.get("archived")).toThrow("已归档");
  expect(service.search({ in_stock_only: false, limit: 1, offset: 1 }).parts).toHaveLength(1);
});

it("keeps electrical constraints unknown and never treats a specification candidate as exact", () => {
  const { service } = fixture();
  const result = service.suggest({ kind: "capacitor", value: "100nF", package: "0402", quantity: 10, constraints: { voltage_rating: "50V" } });
  expect(result.match_kind).toBe("candidate");
  expect(result.candidates[0]).toMatchObject({ requires_confirmation: true, electrical_compatibility: "unverified", sufficient_stock: true });
  expect(result.candidates[0].unknown_parameters).toContain("voltage_rating");
  expect(service.suggest({ lcsc_code: "C999", value: "100nF", package: "0402", quantity: 1 }).candidates).toHaveLength(0);
  expect(service.suggest({ kind: "capacitor", value: "100nF", package: "0603", quantity: 1 }).candidates).toHaveLength(0);
});

it("shares stock across BOM lines, does not allocate candidates and never changes the database", () => {
  const { db, service } = fixture();
  const before = db.prepare("SELECT * FROM parts ORDER BY id").all();
  const result = service.checkBom([{ key: "first", mpn: "R-10K", quantity: 4 }, { key: "second", mpn: "R-10K", quantity: 4 }, { key: "candidate", kind: "capacitor", value: "100nF", package: "0402", quantity: 10 }]);
  expect(result.lines.map((line) => line.exact_identity_covered)).toEqual([4, 1, 0]);
  expect(result.lines[1].candidates[0]).toMatchObject({ available_after_prior_lines: 1, sufficient_stock: false });
  expect(result.uncovered_quantity).toBe(13);
  service.search({ query: "'; DELETE FROM parts; --", limit: 20, offset: 0 });
  expect(db.prepare("SELECT * FROM parts ORDER BY id").all()).toEqual(before);
});

it("does not initialize missing databases or migrate unsupported schemas", () => {
  const { db, path, service } = fixture();
  db.exec("UPDATE schema_migrations SET version=17");
  expect(() => service.get("r")).toThrow("不支持的库存 schema");
  expect(db.prepare("SELECT version FROM schema_migrations").get()?.version).toBe(17);
  const missing = `${path}.missing`;
  expect(() => new InventoryService(missing).search({ limit: 20, offset: 0 })).toThrow();
  expect(existsSync(missing)).toBe(false);
});

it("exposes exactly four read-only tools over a real stdio MCP connection", async () => {
  const { path, db } = fixture();
  const before = db.prepare("SELECT * FROM parts ORDER BY id").all();
  const client = new Client({ name: "partnest-test", version: "1.0.0" });
  const transport = new StdioClientTransport({ command: process.execPath, args: [resolve("dist/index.mjs"), "--database", path], stderr: "pipe" });
  try {
    await client.connect(transport);
    const { tools } = await client.listTools();
    expect(tools.map((tool) => tool.name).sort()).toEqual(["check_bom_stock", "get_inventory_part", "search_inventory", "suggest_inventory_parts"]);
    expect(tools.every((tool) => tool.annotations?.readOnlyHint)).toBe(true);
    for (const [name, args] of [
      ["search_inventory", { query: "C2" }], ["get_inventory_part", { part_id: "c" }],
      ["suggest_inventory_parts", { kind: "capacitor", value: "0.1uF", package: "0402", quantity: 2 }],
      ["check_bom_stock", { lines: [{ key: "R1", mpn: "R-10K", quantity: 2 }] }],
    ] as const) {
      const result = await client.callTool({ name, arguments: args });
      expect(result.isError).not.toBe(true);
      expect(result.structuredContent).toBeTruthy();
    }
    const error = await client.callTool({ name: "get_inventory_part", arguments: { part_id: "missing" } });
    expect(error.isError).toBe(true);
    expect(db.prepare("SELECT * FROM parts ORDER BY id").all()).toEqual(before);
  } finally { await client.close(); }
}, 15_000);
