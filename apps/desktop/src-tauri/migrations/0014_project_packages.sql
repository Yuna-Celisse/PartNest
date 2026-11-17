CREATE TABLE project_packages (
    project_id TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    archive BLOB NOT NULL,
    manifest_json TEXT NOT NULL
);
