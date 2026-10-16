CREATE TABLE IF NOT EXISTS kwants (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    title TEXT NOT NULL,
    persona_prompt TEXT NOT NULL,
    soul TEXT NOT NULL DEFAULT '',
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

CREATE TABLE IF NOT EXISTS teams (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    icon TEXT NOT NULL DEFAULT 'users',
    lead_kwant_id TEXT NOT NULL,
    member_kwants TEXT NOT NULL,
    workspace_path TEXT,
    repo_url TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(lead_kwant_id) REFERENCES kwants(id)
);


CREATE TABLE IF NOT EXISTS mcp_servers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    transport TEXT NOT NULL,
    command TEXT,
    args TEXT NOT NULL DEFAULT '[]',
    url TEXT,
    env TEXT NOT NULL DEFAULT '{}',
    headers TEXT NOT NULL DEFAULT '{}',
    enabled INTEGER NOT NULL DEFAULT 1,
    timeout_secs INTEGER NOT NULL DEFAULT 30,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS kwant_mcp_servers (
    kwant_id TEXT NOT NULL,
    server_id TEXT NOT NULL,
    PRIMARY KEY (kwant_id, server_id),
    FOREIGN KEY(kwant_id) REFERENCES kwants(id) ON DELETE CASCADE,
    FOREIGN KEY(server_id) REFERENCES mcp_servers(id) ON DELETE CASCADE
);

-- Per-kwant opt-outs. Every enabled server is on for every kwant unless a row here turns it off.
-- kwant_mcp_servers (the old opt-in list) is no longer read, but kept so a rollback finds it intact
CREATE TABLE IF NOT EXISTS kwant_mcp_disabled (
    kwant_id TEXT NOT NULL,
    server_id TEXT NOT NULL,
    PRIMARY KEY (kwant_id, server_id),
    FOREIGN KEY(kwant_id) REFERENCES kwants(id) ON DELETE CASCADE,
    FOREIGN KEY(server_id) REFERENCES mcp_servers(id) ON DELETE CASCADE
);

-- OAuth client registrations (RFC 7591), one per server, redirect URI and authorization server
CREATE TABLE IF NOT EXISTS mcp_oauth_clients (
    server_id TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    issuer TEXT NOT NULL,
    client_id TEXT NOT NULL,
    client_secret TEXT,
    auth_method TEXT NOT NULL DEFAULT 'none',
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (server_id, redirect_uri, issuer),
    FOREIGN KEY(server_id) REFERENCES mcp_servers(id) ON DELETE CASCADE
);

-- OAuth tokens for http MCP servers, plus what a refresh needs. Secrets: never serialized
CREATE TABLE IF NOT EXISTS mcp_oauth_tokens (
    server_id TEXT PRIMARY KEY,
    access_token TEXT NOT NULL,
    refresh_token TEXT,
    expires_at TIMESTAMP,
    token_endpoint TEXT NOT NULL,
    client_id TEXT NOT NULL,
    client_secret TEXT,
    auth_method TEXT NOT NULL DEFAULT 'none',
    resource TEXT NOT NULL,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(server_id) REFERENCES mcp_servers(id) ON DELETE CASCADE
);
