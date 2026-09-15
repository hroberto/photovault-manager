-- Esquema inicial do catálogo.
--
-- Duas entidades que não devem ser colapsadas: `object` são bytes identificados por hash,
-- `media` é o item lógico que o usuário chama de foto. Um objeto sustenta um item; um item
-- pertence a vários álbuns e carrega várias pessoas.
--
-- Timestamps são INTEGER epoch em UTC. Nunca TEXT, nunca hora local.

-- Bytes guardados no CAS ------------------------------------------------------
CREATE TABLE object (
    hash_blake3   TEXT    PRIMARY KEY,
    size          INTEGER NOT NULL,
    fidelity      TEXT    NOT NULL,
    stored_at     INTEGER NOT NULL,
    verified_at   INTEGER
) STRICT;

-- Item lógico -----------------------------------------------------------------
-- object_hash é UNIQUE: bytes idênticos são a mesma foto. É daqui que sai, de graça, o
-- comportamento de "o mesmo arquivo visto em quatro álbuns vira um item com quatro
-- associações".
CREATE TABLE media (
    id            INTEGER PRIMARY KEY,
    object_hash   TEXT    NOT NULL UNIQUE REFERENCES object(hash_blake3),
    filename      TEXT    NOT NULL,
    kind          TEXT    NOT NULL,
    source        TEXT    NOT NULL,
    captured_at   INTEGER,
    uploaded_at   INTEGER,
    description   TEXT,
    favorited     INTEGER NOT NULL DEFAULT 0,
    google_url    TEXT,
    import_run_id INTEGER REFERENCES import_run(id),
    edited_from   INTEGER REFERENCES media(id),
    motion_part_of INTEGER REFERENCES media(id),
    last_seen_at  INTEGER,
    deleted_at    INTEGER
) STRICT;

CREATE INDEX idx_media_captured  ON media(captured_at);
CREATE INDEX idx_media_object    ON media(object_hash);
CREATE INDEX idx_media_filename  ON media(filename);

-- Localização -----------------------------------------------------------------
-- Guardamos as duas coordenadas do sidecar quando divergem: `geoData` é o que o usuário vê
-- no Google e pode ter sido editado à mão; `geoDataExif` é o que a câmera gravou.
CREATE TABLE media_place (
    media_id      INTEGER PRIMARY KEY REFERENCES media(id),
    lat           REAL    NOT NULL,
    lon           REAL    NOT NULL,
    altitude      REAL,
    lat_exif      REAL,
    lon_exif      REAL,
    source        TEXT    NOT NULL,
    place_name    TEXT
) STRICT;

-- Pessoas ---------------------------------------------------------------------
-- O Takeout dá os nomes, nunca as coordenadas do rosto. E não há API para devolvê-los ao
-- Google — o valor deste dado está em sobreviver dentro do arquivo, em XMP.
CREATE TABLE person (
    id            INTEGER PRIMARY KEY,
    name          TEXT    NOT NULL,
    name_key      TEXT    NOT NULL UNIQUE,
    notes         TEXT
) STRICT;

CREATE TABLE person_tag (
    media_id      INTEGER NOT NULL REFERENCES media(id),
    person_id     INTEGER NOT NULL REFERENCES person(id),
    source        TEXT    NOT NULL,
    PRIMARY KEY (media_id, person_id)
) STRICT;

CREATE INDEX idx_person_tag_person ON person_tag(person_id);

-- Álbuns ----------------------------------------------------------------------
CREATE TABLE album (
    id            INTEGER PRIMARY KEY,
    title         TEXT    NOT NULL,
    title_key     TEXT    NOT NULL UNIQUE,
    description   TEXT,
    google_album_id TEXT,
    cover_media_id INTEGER REFERENCES media(id),
    created_at    INTEGER
) STRICT;

CREATE TABLE album_media (
    album_id      INTEGER NOT NULL REFERENCES album(id),
    media_id      INTEGER NOT NULL REFERENCES media(id),
    position      INTEGER,
    PRIMARY KEY (album_id, media_id)
) STRICT;

CREATE INDEX idx_album_media_media ON album_media(media_id);

-- Importações -----------------------------------------------------------------
CREATE TABLE import_run (
    id            INTEGER PRIMARY KEY,
    kind          TEXT    NOT NULL,
    source_label  TEXT,
    started_at    INTEGER NOT NULL,
    finished_at   INTEGER,
    media_seen    INTEGER NOT NULL DEFAULT 0,
    media_imported INTEGER NOT NULL DEFAULT 0,
    media_deduplicated INTEGER NOT NULL DEFAULT 0,
    sidecars_matched INTEGER NOT NULL DEFAULT 0,
    sidecars_orphan INTEGER NOT NULL DEFAULT 0,
    media_without_sidecar INTEGER NOT NULL DEFAULT 0,
    bytes_stored  INTEGER NOT NULL DEFAULT 0,
    failures      INTEGER NOT NULL DEFAULT 0
) STRICT;

-- Fila de revisão -------------------------------------------------------------
-- Um sidecar órfão nunca é descartado em silêncio. Se o casamento foi ambíguo, os candidatos
-- ficam registrados para que a revisão humana não precise adivinhar.
CREATE TABLE sidecar_orphan (
    id            INTEGER PRIMARY KEY,
    import_run_id INTEGER NOT NULL REFERENCES import_run(id),
    directory     TEXT    NOT NULL,
    sidecar       TEXT    NOT NULL,
    reason        TEXT    NOT NULL,
    candidates    TEXT,
    resolved_at   INTEGER
) STRICT;

CREATE INDEX idx_orphan_run ON sidecar_orphan(import_run_id);

-- Reimportar o mesmo archive é operação normal e segura: o usuário vai fazer isso quando
-- chegar o segundo Takeout. A fila de revisão não pode crescer a cada passagem.
CREATE UNIQUE INDEX idx_orphan_unique
    ON sidecar_orphan(directory, sidecar)
    WHERE resolved_at IS NULL;

-- Auditoria encadeada ---------------------------------------------------------
-- O encadeamento por hash é o que transforma registro em prova: alterar uma linha antiga
-- invalida todas as seguintes.
CREATE TABLE audit_log (
    id            INTEGER PRIMARY KEY,
    ts            INTEGER NOT NULL,
    operation     TEXT    NOT NULL,
    object_ref    TEXT,
    result        TEXT    NOT NULL,
    details       TEXT,
    prev_hash     TEXT,
    entry_hash    TEXT    NOT NULL
) STRICT;

CREATE INDEX idx_audit_ts ON audit_log(ts);
