ALTER TABLE threads ADD COLUMN config TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(config));
