CREATE TABLE threads (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(trim(id)) > 0 AND instr(id, char(0)) = 0),
    agent TEXT NOT NULL CHECK (agent IN ('claude', 'codex')),
    session_id TEXT NOT NULL CHECK (length(trim(session_id)) > 0 AND instr(session_id, char(0)) = 0),
    cwd TEXT NOT NULL CHECK (substr(cwd, 1, 1) = '/' AND instr(cwd, char(0)) = 0),
    title TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('idle', 'running', 'waiting', 'interrupted', 'failed', 'closed')),
    created_at INTEGER NOT NULL DEFAULT (cast(unixepoch('subsec') * 1000 AS INTEGER)),
    updated_at INTEGER NOT NULL DEFAULT (cast(unixepoch('subsec') * 1000 AS INTEGER))
) STRICT;
CREATE INDEX threads_by_updated ON threads (updated_at DESC, id);

CREATE TABLE messages (
    thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL CHECK (seq > 0),
    role TEXT NOT NULL CHECK (role IN ('user', 'agent', 'system')),
    text TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (cast(unixepoch('subsec') * 1000 AS INTEGER)),
    PRIMARY KEY (thread_id, seq)
) STRICT;

CREATE TABLE requests (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(trim(id)) > 0 AND instr(id, char(0)) = 0),
    thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    payload TEXT NOT NULL CHECK (json_valid(payload)),
    state TEXT NOT NULL CHECK (state IN ('open', 'answered', 'withdrawn')),
    answer TEXT CHECK (answer IS NULL OR json_valid(answer)),
    created_at INTEGER NOT NULL DEFAULT (cast(unixepoch('subsec') * 1000 AS INTEGER)),
    updated_at INTEGER NOT NULL DEFAULT (cast(unixepoch('subsec') * 1000 AS INTEGER)),
    CHECK ((state = 'answered' AND answer IS NOT NULL) OR (state IN ('open', 'withdrawn') AND answer IS NULL))
) STRICT;
CREATE INDEX requests_open ON requests (created_at, id) WHERE state = 'open';
CREATE INDEX requests_by_thread ON requests (thread_id, state, created_at, id);
