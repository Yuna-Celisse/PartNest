CREATE TABLE project_matches (
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    component_key TEXT NOT NULL,
    part_id TEXT NOT NULL REFERENCES parts(id),
    manual INTEGER NOT NULL CHECK(manual IN (0, 1)),
    PRIMARY KEY(project_id, component_key)
);
