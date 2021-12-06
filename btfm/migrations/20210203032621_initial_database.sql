CREATE TABLE IF NOT EXISTS "clips" (
    "uuid" UUID PRIMARY KEY,
    "created_on" TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "last_played" TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "plays" BIGINT NOT NULL DEFAULT 0,
    "phrase" TEXT NOT NULL,
    "description" TEXT NOT NULL,
    "audio_file" TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS "phrases" (
    "uuid" UUID PRIMARY KEY,
    "phrase" TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS "clips_to_phrases" (
    "clip_uuid" UUID NOT NULL,
    "phrase_uuid" UUID NOT NULL,
    PRIMARY KEY(clip_uuid, phrase_uuid),
    FOREIGN KEY (clip_uuid) REFERENCES clips(uuid) ON DELETE CASCADE ON UPDATE NO ACTION,
    FOREIGN KEY (phrase_uuid) REFERENCES phrases(uuid) ON DELETE CASCADE ON UPDATE NO ACTION
);