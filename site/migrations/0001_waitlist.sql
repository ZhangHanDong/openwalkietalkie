-- Separate from Cohdl's list. No IP addresses, browser fingerprints or
-- referral URLs are stored with a signup. Email is normalised by the Worker.
CREATE TABLE IF NOT EXISTS waitlist (
  email TEXT PRIMARY KEY NOT NULL,
  created_at TEXT NOT NULL,
  consent_version TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS waitlist_created_at ON waitlist (created_at);
