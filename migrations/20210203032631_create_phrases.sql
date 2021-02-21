CREATE TABLE IF NOT EXISTS "phrases" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "phrase" VARCHAR(1024) NOT NULL
);

CREATE TABLE IF NOT EXISTS "clips_phrases" (
    "clip_id" INTEGER NOT NULL,
    "phrase_id" INTEGER NOT NULL,
    PRIMARY KEY (clip_id, phrase_id),
    FOREIGN KEY (clip_id) REFERENCES clips(id) ON DELETE CASCADE ON UPDATE NO ACTION,
    FOREIGN KEY (phrase_id) REFERENCES phrases(id) ON DELETE CASCADE ON UPDATE NO ACTION
);

INSERT INTO phrases
    (phrase)
    SELECT phrase
    FROM clips;

INSERT INTO clips_phrases
    (clip_id, phrase_id)
    SELECT clips.id, phrases.id
    FROM clips, phrases
    WHERE clips.phrase=phrases.phrase;