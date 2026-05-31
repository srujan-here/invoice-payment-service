-- Extensions + shared trigger function.
-- citext gives case-insensitive emails without lower() gymnastics.
CREATE EXTENSION IF NOT EXISTS citext;

-- Touch updated_at on every UPDATE. Attached to mutable tables below.
CREATE OR REPLACE FUNCTION set_updated_at() RETURNS trigger AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
