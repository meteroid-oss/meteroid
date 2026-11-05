-- Origins (scheme://host[:port]) the customer portal may be framed in, whatever the license says.
ALTER TABLE tenant ADD COLUMN portal_embed_origins TEXT[] NOT NULL DEFAULT '{}';
