# PartNest Inventory MCP

供 agent 在使用 Konnect 选型前查询真实库存的本地只读 MCP 服务。使用官方 MCP TypeScript SDK v2，支持本地 stdio；不监听网络端口。需要 Node.js 24 或更新版。

## 构建与运行

在仓库根目录运行：

```powershell
pnpm install --frozen-lockfile
pnpm --filter @partnest/inventory-mcp build
node apps/inventory-mcp/dist/index.mjs
```

最后一条命令供 MCP 客户端启动，直接运行会等待 stdin 的 MCP 消息。stdout 仅用于协议，诊断输出在 stderr。

默认数据库为当前用户的 PartNest 数据目录，Windows 为 `%APPDATA%/com.partnest.desktop/partnest.db`。可用 `--database <绝对路径>` 或 `PARTNEST_DATABASE` 环境变量覆盖（命令行优先）。不要使用临时导入目录或备份文件代替实时库存。

## Codex 注册

本机已注册 `partnest-inventory`。在其他机器上，先构建，然后注册绝对路径：

```powershell
codex mcp add partnest-inventory -- node "C:/path/to/PartNest/apps/inventory-mcp/dist/index.mjs" --database "C:/Users/you/AppData/Roaming/com.partnest.desktop/partnest.db"
codex mcp get partnest-inventory
```

若当前聊天尚未出现工具，在 Codex 中刷新 MCP 连接或开启新聊天。Claude 等其他 MCP 客户端也可配置同样的 command/args。

## 工具

| 工具 | 输入示例 | 输出要点 |
| --- | --- | --- |
| `search_inventory` | `{"kind":"capacitor","value":"0.1uF","package":"0402","min_quantity":12}` | 库存列表、总数、分页信息；默认仅有库存且未归档 |
| `get_inventory_part` | `{"part_id":"库存 ID"}` | MPN、LCSC、分类、封装、数量、盒位、版本、更新时间 |
| `suggest_inventory_parts` | `{"kind":"resistor","value":"10k","package":"0402","quantity":8}` | 精确料号或待确认候选、匹配理由、数量覆盖、未知参数，最多 20 个候选 |
| `check_bom_stock` | `{"lines":[{"key":"R1-R8","mpn":"具体料号","quantity":8}]}` | 共用库存余额的 BOM 覆盖计算、每行缺口；每行最多展示 10 个候选 |

检索支持 `query`、`category`、`package`、`kind`、`value`、`min_quantity`、`in_stock_only`、`limit`、`offset`。`query` 对名称、分类、封装、型号、制造商、LCSC 做文字检索；阻容数值等价查询请使用 `value`（如 `0.1uF = 100nF`）。默认每页 20，最多 200。

建议工具可传 `constraints`，例如 `{"voltage_rating":"50V","tolerance":"1%"}`。当前数据库没有这些结构化参数，工具会列入 `unknown_parameters`，不会声称约束已满足。所有响应标明查询时间与 `quantity_semantics: on_hand_not_reserved`。查到的数量只是现有量，不代表为设计预留。

## 与 Konnect 配合

服务启动指令和工具描述会要求 agent 遵循以下流程；工具接入本身不保证所有客户端都会强制执行流程：

1. 放置器件前调用库存检索或建议工具。
2. 优先选择满足设计要求且数量充足的确切 MPN/LCSC，读取详情确认库存 ID 与版本。
3. 数值/封装一致的候选仍需核对耐压、功率、介质、精度、引脚等参数；未知参数不能当作已满足。
4. 再通过 Konnect 查找并校验 KiCad 符号、封装及引脚对应，使用 Konnect 修改设计。库存里的通用封装名不是已验证的 KiCad 库 ID。
5. 在设计交付前调用 BOM 检查，记录库存 ID、MPN、LCSC 和计划用量。

`check_bom_stock` 按输入顺序分配同一库存的余额，避免多行重复计入。只有唯一精确料号命中会计入 `exact_identity_covered`；候选不计入。该数字仅代表料号和数量覆盖，不代表电气兼容性通过。没有预留机制，并行设计仍可能竞争同一库存。实际取用由 PartNest 焊接工作台处理。

## 数据边界

- 每次调用重新打开 SQLite 只读连接，启用 `query_only`，用短读取事务取得一致快照；可读取桌面程序尚未 checkpoint 的 WAL 数据，并避免长期持有恢复前的旧数据库连接。
- 支持 schema 16。版本不支持、数据库缺失或未初始化时返回错误；不创建数据库、不执行迁移、不修改配置、库存、项目或流水。
- 不提供 SQL 执行工具、写入工具或扩展加载能力；只暴露四个固定查询工具。
- 库存名称、备注等内容按不可信数据处理，不应作为 agent 指令执行。

## 验证

```powershell
pnpm --filter @partnest/inventory-mcp test
node apps/inventory-mcp/scripts/smoke.mjs
# 指定数据库：
node apps/inventory-mcp/scripts/smoke.mjs "C:/path/to/partnest.db"
```

测试覆盖 WAL 并发读取、数值/封装归一化、归档与零库存、未知参数、跨 BOM 行重复占用、缺失/不支持数据库以及真实 stdio MCP 调用。smoke 会调用全部四个工具并比较调用前后的库存记录，不做库存调整。
