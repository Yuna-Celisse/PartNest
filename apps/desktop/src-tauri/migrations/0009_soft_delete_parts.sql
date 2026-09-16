-- Archived parts retain their identity and last stock for historical audits.
-- Live inventory queries must exclude deleted rows. Deletion releases location.
ALTER TABLE parts ADD COLUMN deleted_at TEXT;

DROP INDEX parts_lcsc_code_unique;
CREATE UNIQUE INDEX parts_lcsc_code_unique
    ON parts (lcsc_code COLLATE NOCASE)
    WHERE deleted_at IS NULL AND lcsc_code IS NOT NULL AND trim(lcsc_code) <> '';
