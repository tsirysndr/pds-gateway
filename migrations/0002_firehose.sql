-- Per-node firehose progress, so a restart resumes instead of replaying, and
-- the gateway's own sequence never goes backwards.
CREATE TABLE IF NOT EXISTS firehose_cursors (
    node          TEXT PRIMARY KEY,
    upstream_seq  INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

-- Single-row high-water mark for the gateway's sequence numbers.
CREATE TABLE IF NOT EXISTS firehose_sequence (
    id       INTEGER PRIMARY KEY CHECK (id = 1),
    next_seq INTEGER NOT NULL
);
