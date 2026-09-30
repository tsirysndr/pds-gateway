-- Replayable firehose frames. Held on disk so a gateway restart does not force
-- every subscriber back to OutdatedCursor and a full re-crawl.
CREATE TABLE IF NOT EXISTS firehose_frames (
    seq        INTEGER PRIMARY KEY,
    bytes      BLOB NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS firehose_frames_created ON firehose_frames (created_at);
