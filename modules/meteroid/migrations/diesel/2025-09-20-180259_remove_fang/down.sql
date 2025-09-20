create type fang_task_state as enum ('new', 'in_progress', 'failed', 'finished', 'retried');

create table if not exists fang_tasks
(
  id            uuid                     default gen_random_uuid()           not null
    primary key,
  metadata      jsonb                                                        not null,
  error_message text,
  state         fang_task_state          default 'new'::fang_task_state      not null,
  task_type     varchar                  default 'common'::character varying not null,
  uniq_hash     char(64),
  retries       integer                  default 0                           not null,
  scheduled_at  timestamp with time zone default now()                       not null,
  created_at    timestamp with time zone default now()                       not null,
  updated_at    timestamp with time zone default now()                       not null
);

create index if not exists fang_tasks_state_index
  on fang_tasks (state);

create index if not exists fang_tasks_type_index
  on fang_tasks (task_type);

create index if not exists fang_tasks_scheduled_at_index
  on fang_tasks (scheduled_at);

create index if not exists fang_tasks_uniq_hash
  on fang_tasks (uniq_hash);

create table if not exists fang_tasks_archive
(
  id            uuid                     default gen_random_uuid()           not null
    primary key,
  metadata      jsonb                                                        not null,
  error_message text,
  state         fang_task_state          default 'new'::fang_task_state      not null,
  task_type     varchar                  default 'common'::character varying not null,
  uniq_hash     char(64),
  retries       integer                  default 0                           not null,
  scheduled_at  timestamp with time zone default now()                       not null,
  created_at    timestamp with time zone default now()                       not null,
  updated_at    timestamp with time zone default now()                       not null,
  archived_at   timestamp with time zone default now()                       not null
);

create index if not exists fang_tasks_archive_archived_at_index
  on fang_tasks_archive (archived_at);
