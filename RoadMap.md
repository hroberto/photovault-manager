# PhotoVault Manager — RoadMap

**Revisão 2 — 14/09/2026**

---

## Nota desta revisão

A revisão 1 partia de três premissas que não se sustentam. Elas foram verificadas contra a
documentação oficial e corrigidas aqui:

1. **A Data Portability API não cobre o Google Fotos.** Os escopos disponíveis cobrem Chrome,
   Maps, My Activity, My Maps, Play, Saved, Search UGC, Shopping, Street View, YouTube e Fitbit.
   O único escopo com "photos" no nome é `dataportability.maps.photos_videos`, que se refere a
   fotos publicadas no *Maps*. Não existe export programático do acervo do Google Fotos.
2. **O download via Photos API não traz geolocalização.** O `baseUrl` entrega os bytes sem os
   campos de GPS do EXIF, por decisão deliberada da Google. Isso desqualifica a API como fonte
   de preservação fiel.
3. **A deleção programática não existe.** A Library API só gerencia conteúdo criado pelo próprio
   aplicativo.

Mas a revisão 1 também era pessimista demais em um ponto decisivo: **o caminho de volta existe e
é bom**. Com o escopo `photoslibrary.appendonly` é possível reenviar os arquivos para a conta do
usuário, e o Google Fotos lê o EXIF dos bytes recebidos — portanto **data de captura e
geolocalização são restauradas** se estiverem embutidas no arquivo. Álbuns e descrições também
são reconstruíveis via API.

Isso muda o enquadramento do produto. O PhotoVault deixa de ser "backup + expurgo" e passa a ser:

> Um **cofre de ida e volta**: extrai o acervo do Google Fotos com metadados completos, guarda-o
> em formato aberto e auditável, e é capaz de **devolvê-lo** ao Google Fotos — ou a qualquer
> outro destino — sem perda além da que a plataforma impõe.

O expurgo vira consequência, não objetivo: só se apaga com confiança quando a volta é possível.

---

# Parte I — Enquadramento

## 1. Objetivo

Gerenciar, inventariar, preservar e restaurar o acervo de fotos e vídeos do Google Fotos,
mantendo cópia local íntegra, metadados completos e estrutura lógica de álbuns, com capacidade
de reimportação para o Google Fotos.

Cinco responsabilidades:

1. **Exportação** — trazer do Google com o máximo de fidelidade possível
2. **Catalogação** — entender o que se tem
3. **Preservação** — guardar de forma íntegra, aberta e verificável
4. **Restauração** — devolver ao Google Fotos ou a outro destino
5. **Auditoria e limpeza assistida** — saber o que é seguro eliminar

## 2. A realidade das APIs do Google em 2026

Esta seção precisa estar no README desde o primeiro commit, porque tudo depende dela.

| Necessidade | Mecanismo oficial | Situação |
| --- | --- | --- |
| Listar toda a biblioteca | Library API `mediaItems.list` | **Removido** desde 31/03/2025 |
| Selecionar itens existentes | **Picker API** | Disponível, exige seleção manual |
| Baixar bytes com GPS | — | **Não existe via API** |
| Export completo com metadados | **Google Takeout** | Disponível, acionamento manual |
| Export programático | Data Portability API | **Não cobre Google Fotos** |
| Enviar arquivos para a biblioteca | Library API `photoslibrary.appendonly` | **Disponível** |
| Criar álbuns e adicionar itens | Library API | Disponível, só álbuns criados pelo app |
| Definir descrição | `mediaItems.batchCreate` | Disponível |
| Marcar pessoas | — | **Não existe** |
| Marcar favoritos | — | **Não existe** |
| Apagar da biblioteca | — | **Não existe** |

Escopos atuais:

```text
photoslibrary.appendonly                 → enviar arquivos e criar álbuns
photoslibrary.readonly.appcreateddata    → ler o que o próprio app enviou
photoslibrary.edit.appcreateddata        → editar o que o próprio app enviou
photospicker.mediaitems.readonly         → ler o que o usuário selecionou no Picker
```

Os antigos `photoslibrary.readonly` e `photoslibrary` foram removidos do fluxo efetivo.

**Conclusão arquitetural:** a assimetria é o fato central do projeto. **Sair do Google é difícil
e manual; voltar para o Google é fácil e programático.** O desenho inteiro decorre disso.

## 3. As duas direções

```text
         ┌──────────────────────────────────────────────┐
         │              GOOGLE FOTOS                    │
         └───────┬──────────────────────────────▲───────┘
                 │                              │
    EXPORT       │                              │       RESTORE
    (difícil)    │                              │       (fácil)
                 │                              │
        ┌────────▼────────┐            ┌────────┴────────┐
        │  Takeout (ZIP)  │            │  Library API    │
        │  canônico       │            │  appendonly     │
        └────────┬────────┘            └────────▲────────┘
                 │                              │
        ┌────────▼────────┐                     │
        │  Picker API     │                     │
        │  complementar   │                     │
        └────────┬────────┘                     │
                 │                              │
         ┌───────▼──────────────────────────────┴───────┐
         │             PHOTOVAULT                       │
         │   CAS + SQLite + metadados normalizados      │
         └──────────────────────────────────────────────┘
```

O acervo no PhotoVault é a fonte de verdade. O Google Fotos passa a ser **um dos destinos**, não
o dono dos dados.

---

# Parte II — Arquitetura

## 4. Stack

| Camada | Tecnologia |
| --- | --- |
| Linguagem principal | **Rust** |
| Interface | **Tauri 2 + React + TypeScript** |
| Banco local | **SQLite** (SQLx) |
| APIs Google | REST + OAuth 2.0 (PKCE, loopback) |
| Credenciais | OS Keychain, com fallback cifrado |
| Hash | BLAKE3 (identidade), SHA-256 (manifestos) |
| Imagens | image-rs |
| Metadados | rexiv2 / little_exif, ExifTool opcional |
| Vídeos | ffprobe (leitura), FFmpeg opcional |
| Concorrência | Tokio |
| Logs | tracing |
| Testes | cargo test, insta, Playwright |
| Packaging | MSI / DEB / AppImage |

### Por que Rust

Segurança de memória, performance para hashing de dezenas de milhares de arquivos, concorrência
previsível, binário nativo, baixo consumo de memória e código robusto para um software que
manipula dados irrepetíveis. Tauri entrega a experiência de Electron com uma fração do peso.

### Onde o Rust atrapalha, e o que fazer

Seja realista: o ecossistema Rust é fraco em três pontos deste projeto.

- **Escrita de EXIF/XMP.** Nenhuma crate cobre o que o ExifTool cobre. Decisão: usar crates para
  o caminho comum (JPEG/HEIC/PNG) e permitir ExifTool como *backend opcional* detectado em
  runtime, para formatos exóticos e para o modo "máxima fidelidade".
- **Hash perceptual.** Existe, mas imaturo. Fica para depois da V1.
- **CLIP / embeddings.** Exige ONNX Runtime e ~350 MB de modelo no bundle. Plugin opcional, nunca
  no instalador base.

## 5. Arquitetura geral

```text
                     ┌───────────────────────────────┐
                     │        PhotoVault UI          │
                     │  React + TypeScript + Tauri   │
                     └───────────────┬───────────────┘
                                     │  commands / events
                                     ▼
┌─────────────────────────────────────────────────────────────┐
│                     APPLICATION CORE                        │
│                                                             │
│  ImportService     CatalogService      RestoreService       │
│  TakeoutService    MetadataService     AlbumService         │
│  IntegrityService  DuplicateService    AdvisorService       │
│                                                             │
│                      JobEngine (SQLite)                     │
└─────────────────────────────┬───────────────────────────────┘
                              │  traits (ports)
        ┌─────────────────────┼─────────────────────┐
        ▼                     ▼                     ▼
   MediaSource           MediaSink            Repositories
        │                     │                     │
  ┌─────┴──────┐        ┌─────┴──────┐      ┌───────┴───────┐
  │ Takeout    │        │ GooglePhotos│      │ CAS (disco)   │
  │ Picker API │        │ LocalFolder │      │ SQLite        │
  │ LocalFolder│        │ (futuro:    │      │ Keychain      │
  │ (futuro:   │        │  Immich,    │      └───────────────┘
  │  iCloud)   │        │  S3)        │
  └────────────┘        └─────────────┘
```

A novidade em relação à revisão 1 é a porta **`MediaSink`**. Ela é a contraparte simétrica de
`MediaSource` e é o que torna a restauração um conceito de primeira classe em vez de um recurso
avulso.

## 6. Clean / Hexagonal

```text
src/
├── domain/
│   ├── media/          MediaObject, Fidelity, Provenance
│   ├── album/          Album, AlbumMembership
│   ├── person/         Person, PersonTag
│   ├── place/          GeoPoint, Place
│   ├── restore/        RestorePlan, RestoreOutcome
│   └── integrity/      Checksum, VerificationState
│
├── application/
│   ├── import_takeout.rs
│   ├── import_picker.rs
│   ├── normalize_metadata.rs
│   ├── build_restore_plan.rs
│   ├── execute_restore.rs
│   ├── verify_integrity.rs
│   └── build_advisory.rs
│
├── infrastructure/
│   ├── google/
│   │   ├── auth.rs
│   │   ├── picker.rs
│   │   ├── library_upload.rs
│   │   └── drive_takeout.rs
│   ├── takeout/        parser, sidecar matcher, album reader
│   ├── exif/           leitura e escrita
│   ├── cas/
│   ├── sqlite/
│   └── crypto/
│
└── interface/
    └── tauri/
```

## 7. O domínio não conhece o Google

Continua valendo, e agora vale nos dois sentidos.

```rust
/// De onde o acervo vem.
trait MediaSource {
    async fn discover(&self) -> Result<DiscoveryReport>;
    async fn fetch(&self, id: &SourceMediaId) -> Result<MediaStream>;
    fn fidelity(&self) -> Fidelity;
}

/// Para onde o acervo pode voltar.
trait MediaSink {
    async fn capabilities(&self) -> SinkCapabilities;
    async fn upload(&self, object: &StoredObject, meta: &MediaMetadata)
        -> Result<RemoteMediaId>;
    async fn create_album(&self, album: &Album) -> Result<RemoteAlbumId>;
    async fn add_to_album(&self, album: &RemoteAlbumId, items: &[RemoteMediaId])
        -> Result<()>;
}

trait CatalogRepository { /* ... */ }
trait ObjectStore { /* CAS */ }
```

`SinkCapabilities` é o que permite ser honesto com o usuário na tela:

```rust
struct SinkCapabilities {
    supports_description: bool,   // Google: true
    supports_albums: bool,        // Google: true (só álbuns criados pelo app)
    supports_people: bool,        // Google: false
    supports_favorites: bool,     // Google: false
    supports_explicit_geo: bool,  // Google: false (mas lê do EXIF)
    reads_exif_geo: bool,         // Google: true
    max_photo_bytes: u64,         // Google: 200 MB
    max_video_bytes: u64,         // Google: 20 GB
    daily_request_quota: Option<u32>, // Google: 10_000
}
```

A UI deriva dessa struct o aviso de perda antes de qualquer restauração. Nenhuma mensagem é
escrita à mão.

---

# Parte III — EXPORT (Google → PhotoVault)

## 8. Fonte canônica: Takeout

O Takeout é a **única** fonte que entrega o acervo com fidelidade: bytes originais intactos, EXIF
preservado e um JSON lateral por arquivo com o que o Google sabe além do EXIF.

O que ele traz:

```text
Takeout/Google Fotos/
├── Photos from 2019/
│   ├── IMG_1002.JPG
│   ├── IMG_1002.JPG.supplemental-metadata.json
│   ├── IMG_1003.HEIC
│   └── IMG_1003.HEIC.supplemental-metadata.json
├── Viagem Japão/
│   ├── IMG_1002.JPG               ← mesmo arquivo, outra pasta
│   ├── IMG_1002.JPG.supplemental-metadata.json
│   └── metadata.json              ← título e descrição do álbum
└── archive_browser.html
```

Conteúdo típico de um sidecar:

```json
{
  "title": "IMG_1002.JPG",
  "description": "Templo em Kyoto",
  "photoTakenTime": { "timestamp": "1571394000", "formatted": "18/10/2019 09:00:00 UTC" },
  "creationTime": { "timestamp": "1571401200" },
  "geoData":     { "latitude": 35.0116, "longitude": 135.7681, "altitude": 52.0 },
  "geoDataExif": { "latitude": 35.0116, "longitude": 135.7681, "altitude": 52.0 },
  "people": [ { "name": "Henrique" }, { "name": "Ana" } ],
  "favorited": true,
  "googlePhotosOrigin": { "mobileUpload": { "deviceType": "ANDROID_PHONE" } },
  "url": "https://photos.google.com/photo/AF1Qip..."
}
```

**É daqui que saem geolocalização, pessoas, descrições e favoritos.** Nada disso vem pela API.

## 9. Tornando o Takeout suave

O acionamento do export é manual — nenhuma API o dispara. Mas quase todo o resto pode ser
automatizado, e é aí que o produto se diferencia de mandar o usuário se virar.

### Fluxo do assistente

```text
PASSO 1 — Solicitar
   PhotoVault abre o deep link já filtrado só para Fotos:
   https://takeout.google.com/settings/takeout/custom/photos

   Mostra na tela as opções recomendadas:
     Formato       .tgz         (preserva melhor nomes longos que .zip)
     Tamanho       50 GB        (menos arquivos para gerenciar)
     Entrega       Google Drive (ver passo 2)

PASSO 2 — Aguardar
   O Google leva de horas a dias. PhotoVault não fica travado:
   cria um job PENDING_EXTERNAL e libera a interface.

PASSO 3 — Coletar    ← aqui está a suavidade
   (a) Entrega via Drive: com o escopo drive.readonly, o PhotoVault
       LISTA e BAIXA os arquivos takeout-*.tgz sozinho, com retomada.
       O usuário não toca em nada.
   (b) Entrega via link: um watcher na pasta de Downloads detecta
       takeout-*.tgz|zip, valida e ingere automaticamente.
   (c) Manual: arrastar a pasta para a janela.

PASSO 4 — Ingerir
   Extração em streaming, sem descompactar tudo em disco antes.
   Parsing, casamento de sidecars, CAS, catalogação.

PASSO 5 — Conciliar
   Relatório de completude: o que veio, o que faltou, o que não casou.
```

O caminho (a) é o que transforma a experiência. O Takeout entrega no Drive; o Drive **tem** API.
Isso elimina o download manual de oito arquivos de 50 GB — que é a parte genuinamente penosa.

### Conciliação de completude

Terminada a ingestão, o PhotoVault precisa responder "faltou alguma coisa?". Sem
`mediaItems.list` não há contagem oficial, então usa-se triangulação:

```text
Contagem de arquivos de mídia no archive     48.231
Sidecars casados                             48.198
Sidecars órfãos                                  12
Mídias sem sidecar                               33
Álbuns detectados                               184
Itens em álbum sem correspondência                4

Comparar com:
  - a contagem que o usuário lê na tela do Google Fotos (entrada manual)
  - uma amostragem via Picker API
  - o total de bytes informado pelo Google na tela de armazenamento
```

Divergência acima de um limiar vira alerta, não silêncio.

## 10. O parser de Takeout

Este é o componente de maior risco técnico do projeto e merece um crate próprio, corpus de
fixtures reais e testes de propriedade. Sozinho vale metade da V0.1.

Os problemas reais, que não são hipotéticos:

### Nomes dos sidecars

O Google renomeou os sidecars de `IMG_1002.JPG.json` para
`IMG_1002.JPG.supplemental-metadata.json` e trunca o nome final em torno de 46–51 caracteres,
de forma inconsistente. No mesmo archive convivem:

```text
IMG_1002.JPG.supplemental-metadata.json
IMG_1002.JPG.supplemental-metadat.json
IMG_1002.JPG.supplemental-me.json
IMG_1002.JPG.supple.json
IMG_1002.JPG.s.json
IMG_1002.JPG.json                          ← archives antigos
```

Estratégia de casamento, nesta ordem:

```text
1. Correspondência exata
2. Prefixo + qualquer sufixo .supplemental-*.json
3. Casamento por prefixo comum, restrito ao MESMO diretório
4. Desempate por proximidade de nome e por unicidade
5. Órfão → fila de revisão manual, nunca descartado em silêncio
```

### Marcador de duplicata

O `(1)` fica no nome-base, não no fim:

```text
IMG_1002(1).JPG
IMG_1002.JPG(1).supplemental-metadata.json     ← observe a posição
```

### Outras armadilhas

```text
Live Photos           IMG_1002.HEIC + IMG_1002.MP4  → um MediaObject, dois arquivos
Versões editadas      IMG_1002-edited.JPG           → derivada, não original
Mesmo item em N pastas  "Photos from 2019" + cada álbum
Álbuns                 metadata.json no diretório do álbum
Caracteres Unicode     normalização NFC/NFD divergente entre plataformas
Nomes truncados        arquivos longos cortados de forma diferente do sidecar
Fusos                  photoTakenTime em UTC, EXIF em hora local sem offset
```

### Teste

```text
tests/fixtures/takeout/
├── minimal/            5 arquivos, caso feliz
├── truncated_sidecars/ todas as variações de truncamento
├── live_photos/
├── edited_versions/
├── multi_album/
├── unicode_names/
├── duplicate_markers/
└── corrupt/            ZIP truncado, JSON inválido
```

Nenhum parser entra em produção sem passar nesses fixtures.

## 11. O que cada metadado vira

Esta tabela é o contrato do importador.

| Campo do Takeout | Destino no PhotoVault | Destino no arquivo |
| --- | --- | --- |
| bytes | objeto no CAS, imutável | — |
| `photoTakenTime` | `media.captured_at` | `EXIF:DateTimeOriginal` |
| `creationTime` | `media.uploaded_at` | — |
| `geoData.lat/lon/alt` | `place.lat/lon/alt` | `EXIF:GPSLatitude/Longitude/Altitude` |
| `description` | `media.description` | `XMP:Description`, `IPTC:Caption` |
| `people[].name` | tabela `person` + `person_tag` | `XMP-mwg-rs:RegionName`, `XMP:Subject` |
| `favorited` | `media.favorited` | `XMP:Rating` = 5 |
| pasta do álbum | `album` + `album_media` | — |
| `metadata.json` do álbum | `album.title`, `album.description` | — |
| `url` | `media.google_url` | — |
| `googlePhotosOrigin` | `media.origin` | — |

Duas observações que importam:

- **`geoData` vs `geoDataExif`.** Quando divergem, `geoData` é o que o usuário vê no Google (pode
  ter sido editado à mão); `geoDataExif` é o que a câmera gravou. Guardar os dois, usar `geoData`
  como efetivo e registrar a divergência. Quando `geoData` vem zerado e `geoDataExif` não, usar
  o segundo.
- **Pessoas.** O Google dá os nomes, não as coordenadas do rosto. Grava-se o nome como região
  XMP sem retângulo e como keyword. Isso é lido por Lightroom, digiKam e Immich — ou seja, o dado
  sobrevive fora do Google mesmo sem poder voltar para ele.

## 12. Fonte complementar: Picker API

O Picker resolve o que o Takeout não resolve: **o incremento do dia a dia**. Ninguém vai pedir um
Takeout de 390 GB toda semana para capturar as 40 fotos novas.

```text
PhotoVault cria sessão  →  usuário seleciona no Google Fotos  →
itens autorizados       →  download  →  catalogação
```

Serve bem para: lotes recentes, períodos específicos, fotos importantes, migração incremental.

### Mas o Picker é uma fonte degradada

Os bytes vêm **sem GPS no EXIF**. As consequências precisam estar no desenho, não descobertas
depois:

```text
mesmo item, duas origens:
   Takeout  → bytes A → BLAKE3 = a1b2c3...
   Picker   → bytes B → BLAKE3 = f9e8d7...   (A ≠ B: falta o bloco GPS)
```

Sem tratamento, isso produz objetos duplicados no CAS, falsa contagem de "não protegido" e
comparações de integridade sem sentido.

Tratamento:

1. Todo item carrega `fidelity`.
2. A deduplicação usa, além do hash exato, um **hash de conteúdo visual** (hash dos pixels
   decodificados, ignorando metadados) para reconciliar as duas origens.
3. Quando um item `API_DERIVED` é depois recebido via Takeout como `ORIGINAL`, o original
   **substitui** o derivado como objeto canônico e o derivado é descartado.
4. Itens `API_DERIVED` nunca sustentam uma recomendação de expurgo.

## 13. Fidelidade e proveniência

```rust
enum Fidelity {
    /// Bytes originais, metadados completos. Takeout ou pasta local.
    Original,
    /// Bytes obtidos por API, com perda conhecida (GPS removido).
    ApiDerived,
    /// Gerado pelo PhotoVault (normalizado, com EXIF reescrito).
    Normalized,
    /// Derivada de exibição (thumbnail, preview).
    Derivative,
}
```

Regra que atravessa o produto inteiro:

> **Só `Original` é canônico. `ApiDerived` é provisório. `Normalized` é reprodutível e
> descartável.**

## 14. Content Addressable Storage

```text
PhotoVault/
├── repository/
│   └── objects/
│       ├── a1/a1739f4c...          ← bytes originais, IMUTÁVEIS
│       └── b7/b73cc8e1...
│
├── derived/                        ← reconstruível, fora do backup
│   ├── thumbnails/
│   ├── previews/
│   └── normalized/                 ← cópias com EXIF reescrito
│
├── database/
│   └── photovault.db
│
├── metadata/
│   ├── manifests/                  ← manifesto por importação
│   └── exports/
│
└── takeout_archives/               ← opcional: archives originais
```

Mudança em relação à revisão 1: `thumbnails/` saiu de dentro de `repository/`. Misturar dado
canônico com cache reconstruível estraga qualquer estratégia de backup — o `repository/` deve
poder ser copiado sozinho, e o `derived/` deve poder ser apagado sem perda.

### Por que CAS

```text
IMG_1002.JPG aparece em:  Viagem Japão, Família, Favoritas, Photos from 2019

Fisicamente:   1 objeto
Logicamente:   4 associações
```

### A regra que faz o CAS funcionar

> **Objetos no CAS nunca são modificados.**

Se o EXIF corrigido fosse escrito de volta no objeto, o hash mudaria e a identidade se perderia.
Por isso o enriquecimento (seção 16) **sempre gera uma cópia em `derived/normalized/`**, nunca
altera o original. Isso é ADR-004.

## 15. Banco de dados

```sql
-- Conta -----------------------------------------------------------
CREATE TABLE account (
  id INTEGER PRIMARY KEY,
  google_account_id TEXT UNIQUE,
  email TEXT,
  display_name TEXT,
  created_at INTEGER NOT NULL
);

-- Objeto físico (bytes) -------------------------------------------
CREATE TABLE object (
  hash_blake3 TEXT PRIMARY KEY,
  hash_sha256 TEXT,
  size INTEGER NOT NULL,
  path TEXT NOT NULL,
  fidelity TEXT NOT NULL,         -- Original | ApiDerived | Normalized | Derivative
  visual_hash TEXT,               -- hash dos pixels, ignora metadados
  stored_at INTEGER NOT NULL,
  verified_at INTEGER
);

-- Item lógico -----------------------------------------------------
CREATE TABLE media (
  id INTEGER PRIMARY KEY,
  object_hash TEXT NOT NULL REFERENCES object(hash_blake3),
  filename TEXT NOT NULL,
  mime_type TEXT,
  captured_at INTEGER,            -- photoTakenTime
  uploaded_at INTEGER,            -- creationTime no Google
  width INTEGER, height INTEGER, duration_ms INTEGER,
  description TEXT,
  favorited INTEGER DEFAULT 0,
  google_media_id TEXT,
  google_url TEXT,
  origin TEXT,                    -- googlePhotosOrigin
  source TEXT NOT NULL,           -- takeout | picker | local
  provenance_id INTEGER REFERENCES import_run(id),
  live_photo_pair INTEGER REFERENCES media(id),
  edited_from INTEGER REFERENCES media(id),
  last_seen_at INTEGER,
  deleted_at INTEGER
);

-- Metadados técnicos ----------------------------------------------
CREATE TABLE media_exif (
  media_id INTEGER PRIMARY KEY REFERENCES media(id),
  camera_make TEXT, camera_model TEXT, lens TEXT,
  iso INTEGER, exposure TEXT, aperture TEXT, focal_length TEXT
);

-- Lugar -----------------------------------------------------------
CREATE TABLE media_place (
  media_id INTEGER PRIMARY KEY REFERENCES media(id),
  lat REAL, lon REAL, altitude REAL,
  lat_exif REAL, lon_exif REAL,       -- geoDataExif, quando diverge
  source TEXT,                        -- geoData | geoDataExif | user
  place_name TEXT                     -- geocodificação reversa local, opcional
);

-- Pessoas ---------------------------------------------------------
CREATE TABLE person (
  id INTEGER PRIMARY KEY,
  name TEXT UNIQUE NOT NULL,
  notes TEXT
);
CREATE TABLE person_tag (
  media_id INTEGER REFERENCES media(id),
  person_id INTEGER REFERENCES person(id),
  source TEXT NOT NULL,               -- takeout | user
  PRIMARY KEY (media_id, person_id)
);

-- Álbuns ----------------------------------------------------------
CREATE TABLE album (
  id INTEGER PRIMARY KEY,
  google_album_id TEXT,
  title TEXT NOT NULL,
  description TEXT,
  cover_media_id INTEGER REFERENCES media(id),
  created_at INTEGER
);
CREATE TABLE album_media (
  album_id INTEGER REFERENCES album(id),
  media_id INTEGER REFERENCES media(id),
  position INTEGER,
  PRIMARY KEY (album_id, media_id)
);

-- Importações -----------------------------------------------------
CREATE TABLE import_run (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL,                 -- takeout | picker | local
  started_at INTEGER, finished_at INTEGER,
  archive_label TEXT,
  items_seen INTEGER, items_imported INTEGER,
  items_orphan INTEGER, items_failed INTEGER,
  report_path TEXT
);

-- Restauração -----------------------------------------------------
CREATE TABLE restore_run (
  id INTEGER PRIMARY KEY,
  sink TEXT NOT NULL,                 -- google_photos | local_folder
  account_id INTEGER REFERENCES account(id),
  started_at INTEGER, finished_at INTEGER,
  status TEXT,
  items_total INTEGER, items_done INTEGER, items_failed INTEGER
);
CREATE TABLE restore_item (
  restore_run_id INTEGER REFERENCES restore_run(id),
  media_id INTEGER REFERENCES media(id),
  idempotency_key TEXT NOT NULL,
  remote_media_id TEXT,
  status TEXT NOT NULL,               -- pending|uploading|created|failed|skipped
  attempts INTEGER DEFAULT 0,
  last_error TEXT,
  PRIMARY KEY (restore_run_id, media_id)
);
CREATE UNIQUE INDEX idx_restore_idem ON restore_item(idempotency_key);

-- Jobs ------------------------------------------------------------
CREATE TABLE job (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL,
  payload TEXT NOT NULL,              -- JSON
  state TEXT NOT NULL,                -- QUEUED|RUNNING|COMPLETED|FAILED|RETRYING
  attempts INTEGER DEFAULT 0,
  priority INTEGER DEFAULT 0,
  not_before INTEGER,                 -- backoff e janelas de cota
  created_at INTEGER, updated_at INTEGER,
  last_error TEXT
);

-- Auditoria encadeada ---------------------------------------------
CREATE TABLE audit_log (
  id INTEGER PRIMARY KEY,
  ts INTEGER NOT NULL,
  operation TEXT NOT NULL,
  object_ref TEXT,
  result TEXT,
  details TEXT,
  prev_hash TEXT,
  entry_hash TEXT NOT NULL            -- BLAKE3(prev_hash || campos)
);
```

O encadeamento do `audit_log` é barato e é o que transforma "registro" em "prova". Sem ele, um
log de auditoria não demonstra nada.

## 16. Normalização: sidecar → arquivo

Este é o passo que dá valor duradouro ao acervo e que **viabiliza a restauração**.

```text
objeto original (imutável)  +  metadados do SQLite
                    ↓
        derived/normalized/<hash>.jpg
                    ↓
   EXIF:DateTimeOriginal   ← photoTakenTime
   EXIF:GPS*               ← geoData
   XMP:Description         ← description
   XMP-mwg-rs:RegionName   ← people[]
   XMP:Subject             ← people[] como keywords
   XMP:Rating = 5          ← favorited
```

Três propriedades importantes:

1. **É reprodutível.** Apagar `derived/` e regerar dá o mesmo resultado.
2. **É verificável.** O PhotoVault relê o arquivo gerado e confere que os campos entraram.
3. **É a moeda de troca.** É esse arquivo que sobe para o Google, que vai para o Immich, que abre
   no Lightroom com a localização certa. O acervo passa a ser autossuficiente: os metadados
   viajam dentro dos arquivos, não em um banco proprietário.

Para vídeos, a escrita de metadados é mais limitada. Grava-se o que o contêiner aceita
(`creation_time`, `location` em MP4/MOV) e o restante permanece apenas no catálogo.

---

# Parte IV — RESTORE (PhotoVault → Google Fotos)

## 17. O que volta, e o que não volta

Nenhuma tela de restauração deve começar antes que esta tabela esteja implementada como código
(`SinkCapabilities`) e exibida ao usuário.

| Dado | Takeout traz? | Volta ao Google? | Mecanismo |
| --- | :---: | :---: | --- |
| Bytes originais | sim | **sim** | upload direto |
| Data de captura | sim | **sim** | lida do `EXIF:DateTimeOriginal` embutido |
| Geolocalização | sim | **sim** | lida do `EXIF:GPS*` embutido |
| Descrição | sim | **sim** | `mediaItems.batchCreate.description` |
| Álbuns e títulos | sim | **sim** | `albums.create` + `batchAddMediaItems` |
| Ordem dentro do álbum | sim | aproximada | ordem de inserção |
| Dados de câmera (EXIF) | sim | **sim** | embutido no arquivo |
| Nomes de pessoas | sim | **não** | não existe API; o Google redetecta rostos sem os nomes |
| Favoritos | sim | **não** | não existe API |
| Nome do local | derivado | **não** | o Google re-deriva a partir do GPS |
| Data do upload original | sim | não | passa a ser a data do reenvio |
| Itens na lixeira | não | — | o Takeout não os exporta |
| Conversas e curtidas | não | — | fora do escopo |

**O saldo é bom:** o que define uma foto — pixels, quando, onde, o que é, em que álbum está —
volta inteiro. O que se perde são duas camadas de conveniência do Google (nomes de rostos e
estrelas), e o PhotoVault as preserva localmente em XMP, de modo que continuam existindo mesmo
que o Google não as aceite de volta.

A geolocalização voltar é o ponto não óbvio e é o que torna o recurso viável: o Google Fotos lê
o EXIF dos arquivos que recebe, então basta que o passo de normalização (seção 16) tenha
embutido o `geoData` no arquivo antes do envio.

## 18. Pipeline de restauração

```text
RestorePlan
     │
     ▼
seleção de itens  ──►  normalização (se ainda não houver)
     │                        │
     │                        ▼
     │                 verificação do EXIF embutido
     │                        │
     ▼                        ▼
┌──────────────────────────────────────────────┐
│  para cada item                              │
│                                              │
│  1. POST /v1/uploads        (bytes)          │
│     → upload token                           │
│     resumable para vídeos grandes            │
│                                              │
│  2. acumula em lote de 50                    │
│                                              │
│  3. POST /v1/mediaItems:batchCreate          │
│     { uploadToken, fileName, description }   │
│     → mediaItem.id                           │
│                                              │
│  4. grava restore_item.remote_media_id       │
└──────────────────────────────────────────────┘
     │
     ▼
álbuns:  albums.create  →  albums.batchAddMediaItems (50 por chamada)
     │
     ▼
verificação:  mediaItems.get nos itens criados (escopo appcreateddata)
     │
     ▼
relatório de restauração + entrada no audit_log
```

O escopo `photoslibrary.readonly.appcreateddata` permite reler o que o próprio app enviou. É
assim que a restauração se verifica em vez de assumir sucesso.

## 19. Álbuns

```rust
// 1. criar
let remote = google.create_album(&album).await?;   // albums.create

// 2. adicionar em lotes de 50, respeitando album_media.position
for chunk in items.chunks(50) {
    google.add_to_album(&remote, chunk).await?;    // albums.batchAddMediaItems
}
```

Duas limitações a expor na UI:

- O app só adiciona itens a **álbuns que ele mesmo criou**. Restaurar para dentro de um álbum
  preexistente do usuário é impossível. Na prática isso é aceitável: a restauração recria a
  estrutura, não a mescla.
- A capa do álbum não é definível via API. O Google escolhe.

## 20. Limites operacionais

Estes números determinam o desenho, não são nota de rodapé.

```text
Cota de requisições      10.000 / dia / projeto
Cota de bytes (leitura)  75.000 / dia / projeto
batchCreate              50 itens por chamada
Foto                     máx. 200 MB
Vídeo                    máx. 20 GB
Arquivos > 25 MB         contam na cota de armazenamento da conta
Erro de cota             429 → backoff exponencial
```

### A conta que precisa aparecer na tela

Para um acervo de 48.231 fotos e 3.921 vídeos:

```text
uploads                 52.152 requisições
batchCreate              1.044 requisições   (52.152 / 50)
albums.create               184 requisições
batchAddMediaItems        ~1.200 requisições
verificação              ~1.044 requisições
                        ─────────────────────
total                   ~55.600 requisições

a 10.000/dia  →  ~6 dias de execução
```

Consequências de projeto, todas obrigatórias:

1. **A restauração é um job de vários dias.** A engine de jobs (seção 25) precisa sobreviver a
   desligamento, queda de rede e reinício do aplicativo. Não é um botão que roda numa sessão.
2. **Rate limiter com orçamento diário**, usando `job.not_before` para reagendar quando a cota do
   dia acaba. O usuário vê "cota esgotada, retoma em 7h", não um erro.
3. **A UI mostra a estimativa antes de começar**, em dias, não em porcentagem.
4. **Aviso de armazenamento.** Reenviar 390 GB consome 390 GB da cota do Google **de novo** se os
   originais ainda estiverem lá. Este aviso precisa ser intransponível na tela de confirmação.

## 21. Idempotência e anti-duplicata

Um job de seis dias vai ser interrompido. Reiniciar não pode significar reenviar.

```text
idempotency_key = BLAKE3( account_id || object_hash || sink || restore_run_id )
```

Antes de cada upload, consulta-se `restore_item`. Já existe `remote_media_id`? Pula.

Camadas adicionais de proteção:

```text
1. restore_item com UNIQUE em idempotency_key
2. registro do remote_media_id imediatamente após o batchCreate,
   na mesma transação
3. modo simulação (dry-run) obrigatório antes da primeira execução real
4. checagem de colisão: se o item já foi restaurado nesta conta
   em outro restore_run, avisa e pede confirmação explícita
```

O PhotoVault **não** consegue verificar se o item já existe na biblioteca do usuário por outro
caminho — não há como listar a biblioteca. Portanto a proteção contra duplicatas é inteiramente
responsabilidade do catálogo local. Isso precisa estar documentado, porque é uma forma de o
usuário se machucar.

## 22. Para que serve a restauração

Vale enumerar, porque justifica o esforço:

```text
Migração de conta          conta antiga  →  PhotoVault  →  conta nova
Recuperação de desastre    exclusão acidental ou conta perdida
Reidratação seletiva       devolver só um álbum, só um ano
Correção em massa          corrigir datas/locais no PhotoVault e devolver
Saída do Google            PhotoVault → Immich, Nextcloud, disco (outro MediaSink)
Verificação de integridade prova, na prática, que o backup é utilizável
```

O último é o mais importante do ponto de vista conceitual:

> **Um backup que nunca foi restaurado não é um backup — é uma esperança.**

A restauração deixa de ser um recurso e passa a ser o **teste** do produto inteiro. Uma
restauração de amostra (por exemplo, 20 itens aleatórios para um álbum temporário) deveria rodar
periodicamente e alimentar o indicador de saúde do cofre.

---

# Parte V — Preservação

## 23. Estados e integridade

Não marcar `BACKED_UP = true` só porque o download terminou.

```text
DISCOVERED  →  FETCHED  →  HASHED  →  STORED  →  VERIFIED  →  REDUNDANT
```

`VERIFIED` significa: o objeto foi reaberto do disco depois de escrito e o hash bate.
`REDUNDANT` significa: existe em pelo menos duas mídias distintas.

O que mudou em relação à revisão 1: **`VERIFIED` não pode significar "idêntico ao que está no
Google"**, porque o Google não expõe hash e o Picker altera os bytes. É verificação de
integridade local, e a interface precisa dizer exatamente isso. Prometer mais seria mentira.

O que substitui a verificação contra o Google é a **verificação por restauração** (seção 22): a
prova de que o acervo está bom é conseguir devolvê-lo.

## 24. Regra fundamental

Nunca:

```text
download → delete
```

Sempre:

```text
fetch → checksum → persist → reopen → checksum → catalog →
verificação de redundância → (opcional) restauração de amostra →
só então: candidato a expurgo
```

## 25. Jobs

Toda operação longa é um job persistido no SQLite, retomável após desligamento.

```text
Tipos:
  takeout.download      takeout.extract     takeout.parse
  media.hash            media.exif          media.thumbnail
  media.normalize       dedupe.scan         integrity.verify
  restore.upload        restore.album       restore.verify

Estados:
  QUEUED → RUNNING → COMPLETED
              ├──→ FAILED
              └──→ RETRYING (backoff exponencial, not_before)
```

Reabrir o aplicativo retoma exatamente de onde parou. Isso não é um refinamento: com jobs de seis
dias, é requisito.

## 26. Performance

```text
              ┌─ worker 1 ─┐
discovery ────┼─ worker 2 ─┼──► hash ──► exif ──► store
              ├─ worker 3 ─┤
              └─ worker 4 ─┘

bounded channels + backpressure + async I/O
```

Arquivos grandes nunca passam inteiros pela RAM:

```text
stream HTTP / leitura em disco
        ↓
buffer 1–4 MB
        ↓
   ┌────┴────┐
   ▼         ▼
arquivo   BLAKE3 + SHA-256 incrementais
```

Download e hash no mesmo passo, uma leitura só.

## 27. Deduplicação

```text
Nível 1  hash exato (BLAKE3)
         bytes idênticos

Nível 2  hash visual
         mesmos pixels, metadados diferentes
         ← é este que reconcilia Takeout vs Picker

Nível 3  hash perceptual (pHash/dHash)
         redimensionadas, recomprimidas
         pós-V1

Nível 4  embeddings (CLIP)
         rajadas, quase-iguais
         plugin opcional, nunca no instalador base
```

O nível 2 subiu de prioridade nesta revisão: ele deixou de ser refinamento e virou requisito de
correção, por causa da assimetria de bytes entre as duas fontes.

## 28. Hashing

```text
BLAKE3    identidade interna do CAS, por velocidade
SHA-256   calculado sob demanda, para manifestos e interoperabilidade
```

Revisão 1 calculava os dois sempre. Não se justifica: não há hash de referência remoto para
comparar, então o SHA-256 só importa quando o acervo sai do PhotoVault.

## 29. Redundância 3-2-1

```text
Backup Health

Vault local (SSD)     ✓   VERIFIED     378 GB
NAS                   ✓   VERIFIED     378 GB
Off-site              ✓   VERIFIED     378 GB

3 cópias · 2 mídias · 1 fora do local
Última restauração de amostra: 12/09/2026 · 20/20 itens · OK

Status: PROTEGIDO
```

A linha da restauração de amostra é o que diferencia este painel de um indicador decorativo.

## 30. Credenciais

Nunca em texto aberto.

```text
Windows   Credential Manager
macOS     Keychain
Linux     Secret Service (gnome-keyring / KWallet)
```

**Fallback obrigatório:** em Linux headless, NAS ou container o Secret Service não existe. Sem
alternativa, o aplicativo simplesmente não funciona nesses ambientes. O fallback é um arquivo
cifrado com chave derivada de senha mestra (Argon2id), escolhido explicitamente pelo usuário.

O banco guarda `credential_reference`, nunca o `refresh_token`.

## 31. SQLite

```sql
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
PRAGMA synchronous = NORMAL;
PRAGMA busy_timeout = 5000;
```

Migrações versionadas com `sqlx::migrate!`, nunca criação implícita de schema.

---

# Parte VI — Limpeza assistida

## 32. Cleanup Advisor, não Cleanup Executor

A Google não oferece deleção programática. Insistir em "expurgo" como pilar do produto é prometer
o que não se pode entregar. O componente é rebaixado e renomeado.

O que o PhotoVault **faz**:

```text
Analisa, mede, classifica risco, prova que existe backup,
gera a lista, exporta, e leva o usuário até o item no Google Fotos.
```

O que ele **não faz**:

```text
Apagar nada da conta do usuário.
```

A separação entre `AdvisoryEngine` e um eventual `CleanupExecutor` continua sendo a decisão mais
importante desta área: o motor mantém valor integral mesmo que a Google nunca abra a API.

## 33. Relatório de oportunidade

```text
Oportunidade de limpeza

Duplicatas exatas               8,3 GB     risco BAIXO
Vídeos > 1 GB                    43 GB     risco MÉDIO
Originais já verificados         61 GB     risco BAIXO

Recuperação potencial           112 GB

Pré-condições atendidas:
  backup local verificado       ✓
  cópia redundante              ✓
  restauração de amostra OK     ✓
  proveniência = Original       ✓
  janela de segurança cumprida  ✓

              [ Exportar lista ]  [ Abrir no Google Fotos ]
```

Cada item traz o `google_url` do sidecar, que abre a foto exata no Google Fotos. O usuário apaga
em lote pela interface da Google; o PhotoVault registra a intenção, a janela de segurança e o
resultado no `audit_log` encadeado.

## 34. Janela de segurança

```text
candidato  →  0 / 7 / 15 / 30 dias  →  elegível
```

Nenhum item vira elegível no mesmo dia em que é identificado, exceto por escolha explícita.

## 35. Pré-condição absoluta

```text
Um item só é sugerido para eliminação se:
  fidelity == Original
  state == REDUNDANT
  existe em ≥ 2 mídias
  a última restauração de amostra passou
  a janela de segurança venceu
```

Itens `ApiDerived` nunca são sugeridos, porque lhes falta a geolocalização.

---

# Parte VII — Produto

## 36. Tela principal

```text
┌──────────────────────────────────────────────────────────────┐
│ PhotoVault                                         Henrique  │
├───────────────┬──────────────────────────────────────────────┤
│ Painel        │  COFRE                                       │
│ Fotos         │                                              │
│ Álbuns        │  48.231 fotos   ·   3.921 vídeos             │
│ Pessoas       │  184 álbuns     ·   67 pessoas               │
│ Lugares       │                                              │
│ Duplicatas    │  ████████████████████░  98,4% verificado     │
│ ──────────    │                                              │
│ Importar      │  Original      47.902    Derivado       329  │
│ Restaurar     │                                              │
│ ──────────    │  REDUNDÂNCIA                                 │
│ Integridade   │  Vault ✓   NAS ✓   Off-site ✓                │
│ Auditoria     │  Amostra restaurada 12/09  ·  20/20  OK      │
│ Ajustes       │                                              │
│               │  [ Importar Takeout ]   [ Buscar novas ]     │
└───────────────┴──────────────────────────────────────────────┘
```

"Pessoas" e "Lugares" viram seções de primeira classe — são exatamente os metadados que só o
Takeout entrega e que dão ao PhotoVault algo que a API jamais daria.

## 37. Tela de restauração

```text
┌──────────────────────────────────────────────────────────────┐
│ Restaurar para o Google Fotos                                │
├──────────────────────────────────────────────────────────────┤
│ Destino     henrique@gmail.com                    [ trocar ] │
│ Seleção     Álbum "Viagem Japão"        1.284 itens · 14 GB  │
│                                                              │
│ SERÁ RESTAURADO                                              │
│   arquivos originais        ✓        descrições        ✓     │
│   data de captura           ✓        álbum             ✓     │
│   geolocalização            ✓        dados de câmera   ✓     │
│                                                              │
│ NÃO SERÁ RESTAURADO — o Google não oferece API                │
│   nomes de pessoas (37 marcações)                            │
│   marcações de favorito (112 itens)                          │
│   Os dados permanecem no PhotoVault e embutidos nos arquivos.│
│                                                              │
│ ESTIMATIVA                                                   │
│   1.310 requisições · cota 10.000/dia · conclusão hoje       │
│                                                              │
│ ⚠ Serão consumidos 14 GB da cota de armazenamento do Google. │
│                                                              │
│         [ Simular ]   [ Restaurar ]                          │
└──────────────────────────────────────────────────────────────┘
```

Todo o conteúdo dos dois blocos do meio é gerado a partir de `SinkCapabilities`. Nada é texto
fixo — quando a Google mudar a API, a tela muda sozinha.

## 38. Repositório

```text
photovault/
├── apps/desktop/                 Tauri
├── crates/
│   ├── core/                     domínio
│   ├── takeout/                  parser + fixtures   ← maior risco
│   ├── google/                   auth, picker, upload, drive
│   ├── exif/                     leitura e escrita
│   ├── cas/
│   ├── catalog/                  SQLite
│   ├── restore/
│   ├── advisor/
│   └── cli/                      photovault-cli
├── frontend/
├── migrations/
├── tests/{integration,fixtures}/
├── docs/{architecture,adr,api}/
├── Cargo.toml
└── README.md
```

## 39. ADRs

```text
ADR-001  Rust como linguagem principal
ADR-002  Tauri 2 para a interface
ADR-003  SQLite como catálogo
ADR-004  CAS com objetos imutáveis; metadados nunca reescrevem o original
ADR-005  Local-first, sem servidor
ADR-006  Takeout é a fonte canônica; Picker é complementar e degradado
ADR-007  Fidelity como propriedade de primeira classe
ADR-008  BLAKE3 como identidade; SHA-256 sob demanda
ADR-009  Restauração via appendonly como recurso de primeira classe
ADR-010  Capacidades do destino declaradas em código, nunca em texto de UI
ADR-011  Nenhuma eliminação sem redundância verificada
ADR-012  Advisor, não Executor: o PhotoVault não apaga nada no Google
ADR-013  Metadados embutidos nos arquivos, não apenas no banco
ADR-014  Nenhuma automação de navegador
```

## 40. Releases

A mudança principal em relação à revisão 1: **a interface vem depois da confiança nos dados**, e
a restauração sobe de prioridade porque é o que valida o produto inteiro.

### V0.1 — Ingestão (CLI, sem interface)

```text
photovault import-takeout ./Takeout --vault ~/PhotoVault
photovault status
photovault report --orphans
```

Parser de Takeout com fixtures, CAS, SQLite, catálogo completo com geo, pessoas e álbuns.
Ataca primeiro o maior risco técnico. Se o parser não for confiável, nada mais importa.

### V0.2 — Integridade e normalização

Hash, verificação por releitura, deduplicação por hash exato e visual, normalização
sidecar → EXIF/XMP, manifestos, auditoria encadeada.

### V0.3 — Restauração (ainda CLI)

```text
photovault restore --album "Viagem Japão" --to google --dry-run
photovault restore --album "Viagem Japão" --to google
photovault restore --sample 20 --to google
```

OAuth, upload, batchCreate, álbuns, idempotência, rate limiter, verificação.
**Aqui o produto passa a provar que funciona.**

### V0.4 — Interface

Tauri e React sobre um núcleo já confiável: painel, galeria, álbuns, pessoas, lugares,
assistente de importação, tela de restauração.

### V0.5 — Incremental

Picker API, integração com o Drive para coleta automática dos archives, watcher de pasta,
conciliação de completude.

### V0.6 — Advisor

Análise de armazenamento, candidatos, risco, relatório, janela de segurança, deep links.

### Depois

Hash perceptual, mapa, linha do tempo, outros `MediaSink` (Immich, Nextcloud, S3), busca
semântica local, reconhecimento facial local e opcional.

## 41. Riscos conhecidos

| Risco | Impacto | Mitigação |
| --- | --- | --- |
| Google muda de novo as APIs | alto | domínio isolado por traits; nada da lógica depende do Google |
| Variações do Takeout não previstas | alto | fixtures reais, órfãos nunca descartados em silêncio |
| Reenvio duplica itens na conta | alto | idempotência, dry-run obrigatório, aviso explícito |
| Reenvio consome cota de armazenamento | médio | estimativa antes de executar, confirmação intransponível |
| Cota de 10.000/dia torna a restauração longa | médio | jobs retomáveis, orçamento diário, estimativa em dias |
| Escrita de EXIF imperfeita em Rust | médio | ExifTool como backend opcional; validação por releitura |
| Nomes de pessoas não voltam | baixo | preservados em XMP; declarado na UI |
| Secret Service ausente em Linux | baixo | fallback com senha mestra |
| Escopo grande demais para um projeto pessoal | alto | CLI antes de UI; cada release entrega algo utilizável |

## 42. Princípios

1. O acervo é do usuário; o Google é apenas um dos destinos.
2. O original nunca é modificado.
3. Os metadados viajam dentro dos arquivos, não presos a um banco proprietário.
4. Nada é apagado sem redundância verificada.
5. O que não é possível é dito na tela, não descoberto pelo usuário.
6. Um backup que nunca foi restaurado não é um backup.

---

## Referências

- [Updates to the Google Photos APIs](https://developers.google.com/photos/support/updates) — fim do acesso à biblioteca completa em 31/03/2025
- [Google Photos Picker API](https://developers.google.com/photos/picker/guides/media-items) — seleção e `baseUrl`
- [Upload media (Library API)](https://developers.google.com/photos/library/guides/upload-media) — `appendonly`, limites de 200 MB / 20 GB, 50 por `batchCreate`
- [API limits and quotas](https://developers.google.com/photos/overview/api-limits-quotas) — 10.000 requisições/dia
- [Authorization scopes](https://developers.google.com/photos/overview/authorization) — escopos atuais
- [Data Portability API scopes](https://developers.google.com/data-portability/user-guide/scopes) — confirma a ausência do Google Fotos
- [EXIF metadata missing from API downloads](https://issuetracker.google.com/issues/111228390) — GPS removido, aberto desde 2018
- [Google Takeout JSON files explained](https://metadatafixer.com/learn/google-takeout-json-files-explained) — estrutura dos sidecars
