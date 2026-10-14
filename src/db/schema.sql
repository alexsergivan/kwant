CREATE TABLE IF NOT EXISTS kwants (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    title TEXT NOT NULL,
    persona_prompt TEXT NOT NULL,
    avatar_sprite TEXT NOT NULL,
    driver_type TEXT NOT NULL DEFAULT 'api',
    preferred_model TEXT NOT NULL DEFAULT 'auto',
    temperature REAL NOT NULL DEFAULT 0.7,
    max_tokens INTEGER NOT NULL DEFAULT 4096,
    level INTEGER NOT NULL DEFAULT 1,
    turns_completed INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS channels (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    lead_kwant_id TEXT NOT NULL,
    member_kwants TEXT NOT NULL,
    routing_mode TEXT NOT NULL DEFAULT 'auto',
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(lead_kwant_id) REFERENCES kwants(id)
);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    channel_id TEXT,
    direct_kwant_id TEXT,
    sender_role TEXT NOT NULL,
    sender_id TEXT NOT NULL,
    text TEXT NOT NULL,
    reasoning_content TEXT,
    tool_calls TEXT,
    tokens_used INTEGER DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS approvals (
    id TEXT PRIMARY KEY,
    turn_id TEXT NOT NULL,
    kwant_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    parameters TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
