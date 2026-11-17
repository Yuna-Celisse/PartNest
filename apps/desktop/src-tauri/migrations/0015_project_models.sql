CREATE TABLE project_models (
    project_id TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    glb BLOB NOT NULL
);
