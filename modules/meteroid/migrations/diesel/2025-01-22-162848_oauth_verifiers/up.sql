create table if not exists oauth_verifier (
    id uuid primary key,
    csrf_token text not null,
    pkce_verifier text not null,
    is_signup boolean not null,
    invite_key text,
    created_at timestamp default CURRENT_TIMESTAMP not null
);

create unique index if not exists oauth_verifier_csrf_token_idx on oauth_verifier (csrf_token);
