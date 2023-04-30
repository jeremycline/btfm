CREATE TABLE IF NOT EXISTS "clips" (
    "uuid" TEXT NOT NULL PRIMARY KEY,
    "created_on" TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "last_played" TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "plays" BIGINT NOT NULL DEFAULT 0,
    "speech_detected" TEXT,
    "audio_file" TEXT NOT NULL,
    "original_file_name" TEXT NOT NULL,
    "title" TEXT NOT NULL,
    "description" TEXT
);
CREATE INDEX "clips_title_index" ON "clips" ("title");

CREATE TABLE IF NOT EXISTS "clip_phrases" (
    "uuid" TEXT NOT NULL PRIMARY KEY,
    "clip" TEXT NOT NULL,
    "phrase" TEXT NOT NULL,
    FOREIGN KEY (clip) REFERENCES clips(uuid) ON DELETE CASCADE ON UPDATE NO ACTION
);
