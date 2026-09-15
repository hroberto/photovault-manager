# PhotoVault Manager

Um cofre de ida e volta para o acervo do Google Fotos: extrai com metadados completos, guarda em
formato aberto e auditável, e é capaz de **devolver** ao Google Fotos — ou a qualquer outro
destino.

> O código, os comentários e a documentação estão em português, que é a língua do projeto.

---

## O problema

Desde 31 de março de 2025 a Google restringiu a Library API ao conteúdo criado pelo próprio
aplicativo. Isso muda o que é possível construir, e a maioria das ferramentas ainda parte de
premissas que não valem mais. Os fatos verificados contra a documentação oficial:

| Necessidade | Situação em 2026 |
| --- | --- |
| Listar toda a biblioteca | **Removido** — `mediaItems.list` não alcança mais o acervo do usuário |
| Selecionar itens existentes | Picker API, com seleção manual |
| Baixar bytes com GPS | **Não existe** — a API remove o bloco de geolocalização do EXIF |
| Export completo com metadados | Só Google Takeout, acionamento manual |
| Export programático | Data Portability API **não cobre** o Google Fotos |
| Enviar arquivos para a biblioteca | `photoslibrary.appendonly` — funciona |
| Criar álbuns e adicionar itens | Funciona, apenas em álbuns criados pelo próprio app |
| Marcar pessoas ou favoritos | **Não existe** |
| Apagar da biblioteca | **Não existe** |

Daí decorre a assimetria que organiza o projeto inteiro:

> **Sair do Google é manual e difícil. Voltar para o Google é programático e fácil.**

## A consequência não óbvia

A geolocalização **volta**. O Google Fotos lê o EXIF dos bytes que recebe — então basta embutir
no arquivo, antes de enviar, o `geoData` que o Takeout entregou no JSON lateral. O mesmo vale
para a data de captura.

O que não volta são nomes de pessoas e marcações de favorito, porque não há API para isso. O
PhotoVault preserva ambos em XMP dentro do arquivo, de modo que sobrevivem para Lightroom,
digiKam e Immich mesmo que o Google não os aceite de volta.

---

## Estado

**V0.3** — 249 testes, `clippy -D warnings` limpo.

| Componente | Estado |
| --- | --- |
| `crates/core` — domínio, sem dependência do Google | pronto |
| `crates/takeout` — parser do Takeout | pronto |
| `crates/cas` — armazenamento endereçado por conteúdo | pronto |
| `crates/catalog` — SQLite com SQLx | pronto |
| `crates/exif` — metadados embutidos | pronto |
| `crates/google` — Library API, OAuth, cota | pronto, testado contra servidor simulado |
| `crates/restore` — planejamento, idempotência, retomada | núcleo pronto |
| `crates/cli` — `photovault` | importação, verificação, normalização |
| `crates/advisor` — análise de limpeza | não iniciado |
| interface Tauri | não iniciada |

O que falta para a primeira restauração real é um Client ID OAuth, que só o dono da conta pode
criar.

---

## O que o parser do Takeout resolve

O Takeout não tem especificação e o formato muda sem aviso. Estas armadilhas são tratadas com
fixtures e testes, não com suposições:

```
IMG_1002.JPG.supplemental-metadata.json     formato atual
IMG_1002.JPG.supplemental-metadat.json      truncado
IMG_1002.JPG.supple.json                    truncado em outro ponto
IMG_1002.JPG.s.json                         truncado ao extremo
IMG_1002.JPG.json                           formato antigo

IMG_1002(1).JPG  ←→  IMG_1002.JPG(1).supplemental-metadata.json
                     o marcador de duplicata migra para depois da extensão

Aniversário.jpg  ←→  Aniversa<0301>rio.jpg.…json
                     NFC contra NFD, conforme a plataforma

IMG_1004.HEIC + IMG_1004.MP4     uma Live Photo, não duas fotos
IMG_1003-edited.JPG              derivada, não duplicata (12 idiomas)
```

Quando o casamento é ambíguo, o sidecar vira **órfão com o motivo e os candidatos registrados**.
Nunca um palpite silencioso: associar metadado à foto errada é o modo de falha que mais assusta
num acervo irrepetível.

---

## Princípios

1. O acervo é do usuário; o Google é apenas um dos destinos.
2. O original nunca é modificado — objetos no CAS são gravados em modo `0444`, e a imutabilidade
   é imposta pelo sistema de arquivos, não só documentada.
3. Os metadados viajam dentro dos arquivos, não presos a um banco proprietário.
4. Nada é apagado sem redundância verificada. **O PhotoVault nunca apaga nada no Google.**
5. O que não é possível é dito na tela, não descoberto pelo usuário.
6. Um backup que nunca foi restaurado não é um backup.

As decisões estão registradas em [`docs/adr/`](docs/adr/), com contexto e custo.

---

## Uso

Requer Rust estável.

```bash
cargo build --release

# 1. Solicite o export em takeout.google.com (só Fotos) e extraia o archive.
photovault import-takeout ./Takeout --vault ~/PhotoVault --label "archive 1 de 8"

# 2. Veja o que entrou.
photovault status --vault ~/PhotoVault

# 3. Confira a integridade dos bytes em disco.
photovault verify --vault ~/PhotoVault
photovault verify --sample 2 --vault ~/PhotoVault    # scrub periódico

# 4. Grave os metadados dentro de cópias dos arquivos.
#    Sem isso a geolocalização não volta ao Google.
photovault normalize --vault ~/PhotoVault

# 5. Reveja o que não casou. Nada foi descartado.
photovault orphans --vault ~/PhotoVault
```

### Estrutura do cofre

```
PhotoVault/
├── repository/objects/     bytes originais, imutáveis, endereçados por BLAKE3
├── derived/normalized/     cópias com metadados embutidos — descartável e reprodutível
└── database/photovault.db  catálogo
```

`repository/` pode ser copiado sozinho. `derived/` pode ser apagado sem perda.

---

## Desenvolvimento

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

O crate `takeout` é o de maior risco técnico do projeto. Toda variação encontrada em um archive
real deve virar fixture **antes** de virar correção de código.

---

## Licença

MIT ou Apache-2.0, à escolha de quem usa.
