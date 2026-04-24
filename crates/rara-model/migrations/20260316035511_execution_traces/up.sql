CREATE TABLE IF NOT EXISTS execution_traces (
    -- ULID primary key, sortable by creation time.
    id          TEXT    PRIMARY KEY NOT NULL,
    -- Session that produced this trace.
    session_id  TEXT    NOT NULL,
    -- Full ExecutionTrace as JSON.
    trace_data  TEXT    NOT NULL,
    created_at  TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_execution_traces_session ON execution_traces(session_id);
