# PhotoVault Manager

Cofre de ida e volta para o acervo do Google Fotos: extrai com metadados completos, guarda em
formato aberto e auditável, e é capaz de devolver ao Google Fotos ou a outro destino.

O plano completo está em `RoadMap.md`. As decisões estão em `docs/adr/`.

## Fatos que governam o projeto

Verificados contra a documentação oficial. Não os contradiga sem verificar de novo.

- `mediaItems.list` da biblioteca foi **removido** em 31/03/2025. Não existe listar o acervo.
- Download pela Photos API vem **sem GPS** no EXIF. Por isso o Takeout é a fonte canônica.
- A **Data Portability API não cobre o Google Fotos**. Não existe export programático.
- **Não existe API de deleção.** O PhotoVault nunca apaga nada no Google (ADR-012).
- O caminho de volta **existe**: `photoslibrary.appendonly` envia arquivos e cria álbuns, e o
  Google lê o EXIF dos bytes recebidos — então data e geolocalização voltam.
- Nomes de pessoas e favoritos **não voltam**. Não há API.
- Cota: 10.000 requisições/dia. Restaurar 52 mil itens leva ~6 dias.

## Regras invioláveis

1. **Objetos no CAS são imutáveis.** Metadado nunca reescreve o original; gera cópia em
   `derived/normalized/`. (ADR-004)
2. **Nada é apagado sem redundância verificada.** (ADR-011)
3. **Duplicata não é removida sem migrar álbuns, pessoas, descrição e favorito para quem fica.**
4. **Capacidades do destino vivem em código** (`SinkCapabilities`), nunca em texto de UI. (ADR-010)
5. **Órfão de sidecar nunca é descartado em silêncio.**
6. **Credencial só no keychain.** O SQLite guarda referência, jamais o token.
7. **Sem `unwrap`/`expect`** fora de testes.
8. **Sem automação de navegador.** (ADR-014)

## Estrutura

```
crates/core/       domínio, sem dependência de Google
crates/takeout/    parser do Takeout  ← maior risco técnico
crates/google/     auth, picker, upload, drive
crates/exif/       escrita e verificação de metadados embutidos
crates/cas/        content addressable storage
crates/catalog/    SQLite + SQLx
crates/restore/    restauração
crates/advisor/    análise de limpeza
crates/cli/        photovault-cli
apps/desktop/      Tauri
frontend/          React + TypeScript
```

## Ordem de construção

CLI antes de interface. O parser de Takeout antes de tudo: se ele não for confiável, nada mais
importa. Ver `RoadMap.md` seção 40.

## Comandos

```bash
cargo test                          # todos os testes
cargo test -p photovault-takeout    # só o parser
cargo clippy --all -- -D warnings   # obrigatório antes de considerar pronto
cargo run -p photovault-cli -- import-takeout ./Takeout --vault ~/PhotoVault
cargo run -p photovault-cli -- normalize --vault ~/PhotoVault
cargo run -p photovault-cli -- verify --sample 2 --vault ~/PhotoVault
cargo run -p photovault-cli -- orphans --vault ~/PhotoVault
```

## Estado atual (V0.3)

Pronto e testado: domínio, parser do Takeout, CAS, catálogo, CLI de importação,
detecção de parentesco, normalização de metadados, cliente da Library API
(testado contra servidor simulado) e o núcleo da restauração — planejamento,
orçamento de cota, idempotência e retomada. 249 testes.

Falta apenas o fio final: ligar o cliente à fila de restauração num comando da
CLI. Isso exige um Client ID OAuth, que só o usuário pode criar no Google Cloud
Console (Photos Library API + Photos Picker API, aplicativo para computador).

Ainda não iniciados: `crates/advisor` e a interface Tauri (esta também precisa
de Node, ausente na máquina).

Limite conhecido do `crates/exif`: o backend nativo grava EXIF, não XMP. Nomes de pessoas
e favoritos são reportados como não embutidos, com o remédio (ExifTool). Nunca são
descartados em silêncio.

## Especialistas

Há perfis em `.claude/agents/` para as áreas de risco: `takeout-forensics`, `rust-core`,
`google-photos-api`, `media-metadata`, `sqlite-catalog`, `vault-security`, `tauri-frontend`.
