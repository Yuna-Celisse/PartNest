-- 一个项目可以同时具备两样来源：交互式画布（HTML）与器件表格（CSV/XLS/XLSX）。
-- 导入交互式 BOM 时附带表格、或之后补充导入表格，都会让项目变得「完整」；
-- 表格项目本身就带器件表格，只有画布需要事后补充。
ALTER TABLE projects ADD COLUMN has_table INTEGER NOT NULL DEFAULT 0;

UPDATE projects SET has_table = 1 WHERE kind = 'tabular';
