alter table oauth_verifier drop column data;
alter table oauth_verifier add column is_signup boolean not null default false;
alter table oauth_verifier add column invite_key text;
