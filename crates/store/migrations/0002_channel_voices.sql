ALTER TABLE channels ADD COLUMN voices TEXT NOT NULL DEFAULT '[]';
ALTER TABLE channels ADD COLUMN voice_namespace TEXT;
