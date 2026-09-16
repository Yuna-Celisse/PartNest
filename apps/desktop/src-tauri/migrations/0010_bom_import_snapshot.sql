-- Imported BOMs are stored as parsed snapshots so the history can be reopened
-- from the database alone, without the original file or a cache copy.
-- `kind` distinguishes tabular imports (csv/xlsx) from interactive HTML ones.
ALTER TABLE bom_files ADD COLUMN kind TEXT NOT NULL DEFAULT 'interactive'
    CHECK (kind IN ('interactive', 'tabular'));
ALTER TABLE bom_files ADD COLUMN normalized_json TEXT;