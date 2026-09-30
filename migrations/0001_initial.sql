-- Accounts the gateway knows about, and which node hosts each one.
-- `handle` is UNIQUE and case-insensitive: this constraint is what makes two
-- nodes unable to both claim `alice.rocksky.social`.
CREATE TABLE IF NOT EXISTS accounts (
    did         TEXT PRIMARY KEY,
    handle      TEXT NOT NULL COLLATE NOCASE,
    node        TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'active',
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS accounts_handle_unique ON accounts (handle COLLATE NOCASE);
CREATE INDEX IF NOT EXISTS accounts_node ON accounts (node);

-- Handles held while an account creation is in flight on an upstream node.
CREATE TABLE IF NOT EXISTS handle_reservations (
    handle      TEXT PRIMARY KEY COLLATE NOCASE,
    node        TEXT NOT NULL,
    owner       TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS handle_reservations_expiry ON handle_reservations (expires_at);

-- Learned identifier -> node mappings for identifiers the gateway cannot
-- resolve itself, chiefly the email addresses used to log in.
CREATE TABLE IF NOT EXISTS identifier_hints (
    identifier  TEXT PRIMARY KEY COLLATE NOCASE,
    node        TEXT NOT NULL,
    updated_at  INTEGER NOT NULL
);
