-- No delivery private key is stored: it is derived from the service secret and id.
CREATE TABLE email_invitation (
    id TEXT PRIMARY KEY,
    sender TEXT NOT NULL,
    email TEXT NOT NULL,
    space TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    original TEXT,
    recipient TEXT,
    completed TEXT,
    sent_at INTEGER,
    cancelled INTEGER NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at INTEGER NOT NULL DEFAULT 0,
    CHECK ((recipient IS NULL) = (completed IS NULL))
);
CREATE INDEX email_invitation_sender_time ON email_invitation(sender, created_at);
CREATE INDEX email_invitation_email_time ON email_invitation(email, created_at);
