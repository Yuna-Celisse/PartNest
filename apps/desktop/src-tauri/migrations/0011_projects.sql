-- 项目成为顶层实体：一次导入（交互式 BOM 或表格 BOM）就是一个项目，焊接工作台按项目打开。
-- bom_files 的一行本来就完整地描述了一个项目的 BOM 快照，所以这里只做重命名升级，
-- 主键与会话 id 全部原样保留，历史流水的项目归属因此不需要任何回补。
ALTER TABLE bom_files RENAME TO projects;

-- 「项目名」取代了 BOM 备注名：它由导入时的文件名得到，之后可以在项目列表里改。
ALTER TABLE projects RENAME COLUMN display_name TO name;

-- 会话归属从「BOM 文件」变成「项目」；外键定义在 RENAME TABLE 时已指向 projects。
ALTER TABLE welding_sessions RENAME COLUMN bom_file_id TO project_id;
