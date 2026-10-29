-- One OAuth authorization, from `/authorize` until `/token` redeems its
-- code. Every secret is held as a hash: the request handle the approval
-- page carries, the browser cookie bound to it, and the code. The token
-- itself is sealed under a key derived from the code, so a row read
-- without the code (a backup, Time Travel) opens nothing.
CREATE TABLE oauth_authorization (
    request TEXT PRIMARY KEY,
    browser TEXT NOT NULL,
    client_id TEXT NOT NULL,
    client_name TEXT NOT NULL,
    registration TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    state TEXT NOT NULL,
    resource TEXT NOT NULL,
    challenge TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    code TEXT UNIQUE,
    token TEXT,
    CHECK ((code IS NULL) = (token IS NULL))
);
CREATE INDEX oauth_authorization_expiry ON oauth_authorization(expires_at);
