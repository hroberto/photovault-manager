-- Restauração e engine de jobs.
--
-- Uma restauração de 52 mil itens leva cerca de seis dias na cota de 10.000 requisições
-- diárias. Isso não é um botão que roda numa sessão: é um trabalho que precisa sobreviver a
-- reinício do aplicativo, queda de rede e fim de cota. Por isso o estado vive no banco.

-- Conta do destino --------------------------------------------------------
-- Guarda apenas a identidade e uma referência à credencial. O refresh_token vive no chaveiro
-- do sistema operacional, nunca aqui (ADR de segurança em .claude/agents/vault-security.md).
CREATE TABLE account (
    id                   INTEGER PRIMARY KEY,
    provider             TEXT    NOT NULL,   -- google
    external_id          TEXT,
    email                TEXT    NOT NULL,
    display_name         TEXT,
    credential_reference TEXT,
    granted_scopes       TEXT,
    created_at           INTEGER NOT NULL,
    last_used_at         INTEGER,
    UNIQUE (provider, email)
) STRICT;

-- Uma execução de restauração ---------------------------------------------
CREATE TABLE restore_run (
    id            INTEGER PRIMARY KEY,
    sink          TEXT    NOT NULL,          -- google_photos | local_folder
    account_id    INTEGER REFERENCES account(id),
    started_at    INTEGER NOT NULL,
    finished_at   INTEGER,
    status        TEXT    NOT NULL,          -- planning|running|paused|done|failed|cancelled
    dry_run       INTEGER NOT NULL DEFAULT 0,
    items_total   INTEGER NOT NULL DEFAULT 0,
    items_done    INTEGER NOT NULL DEFAULT 0,
    items_failed  INTEGER NOT NULL DEFAULT 0,
    items_skipped INTEGER NOT NULL DEFAULT 0,
    bytes_total   INTEGER NOT NULL DEFAULT 0,
    requests_estimated INTEGER NOT NULL DEFAULT 0,
    note          TEXT
) STRICT;

-- Um item dentro de uma restauração ---------------------------------------
-- A chave de idempotência é o que torna seguro reiniciar um trabalho de seis dias. Antes de
-- gastar uma requisição, consulta-se esta tabela: já há remote_media_id? Pula.
--
-- O índice é UNIQUE e global, não por execução: se o mesmo objeto já foi enviado para a mesma
-- conta em outra execução, o conflito aparece aqui. É a única proteção contra duplicar itens
-- na biblioteca do usuário, porque não existe API para listar o que já está lá.
CREATE TABLE restore_item (
    restore_run_id  INTEGER NOT NULL REFERENCES restore_run(id),
    media_id        INTEGER NOT NULL REFERENCES media(id),
    idempotency_key TEXT    NOT NULL,
    normalized_path TEXT,
    upload_token    TEXT,
    remote_media_id TEXT,
    status          TEXT    NOT NULL,        -- pending|uploading|uploaded|created|failed|skipped
    attempts        INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,
    updated_at      INTEGER,
    PRIMARY KEY (restore_run_id, media_id)
) STRICT;

CREATE UNIQUE INDEX idx_restore_idempotency ON restore_item(idempotency_key);
CREATE INDEX idx_restore_status ON restore_item(restore_run_id, status);

-- Álbuns recriados no destino ---------------------------------------------
-- O aplicativo só consegue adicionar itens a álbuns que ele mesmo criou, então guardamos o
-- identificador remoto para reencontrá-lo ao retomar.
CREATE TABLE restore_album (
    restore_run_id  INTEGER NOT NULL REFERENCES restore_run(id),
    album_id        INTEGER NOT NULL REFERENCES album(id),
    remote_album_id TEXT,
    items_added     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (restore_run_id, album_id)
) STRICT;

-- Engine de jobs ----------------------------------------------------------
CREATE TABLE job (
    id          INTEGER PRIMARY KEY,
    kind        TEXT    NOT NULL,
    payload     TEXT    NOT NULL,            -- JSON
    state       TEXT    NOT NULL,            -- queued|running|completed|failed|retrying
    attempts    INTEGER NOT NULL DEFAULT 0,
    priority    INTEGER NOT NULL DEFAULT 0,
    not_before  INTEGER,                     -- backoff e janela de cota
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    last_error  TEXT
) STRICT;

CREATE INDEX idx_job_ready ON job(state, not_before, priority);

-- Consumo de cota ---------------------------------------------------------
-- A cota do Google é por projeto e por dia, em horário do Pacífico. Contamos localmente para
-- poder dizer "retoma em 7h" em vez de bater num 429 e falhar.
CREATE TABLE quota_usage (
    day         TEXT    PRIMARY KEY,         -- AAAA-MM-DD no fuso da cota
    requests    INTEGER NOT NULL DEFAULT 0,
    updated_at  INTEGER NOT NULL
) STRICT;
