# ADR-013: Metadados embutidos nos arquivos

**Estado:** Aceito
**Data:** 2026-09-14

## Contexto

Um acervo cujo significado só existe em um banco SQLite proprietário morre junto com o software
que o criou. Além disso, o Google só recupera data e geolocalização lendo o EXIF dos bytes que
recebe — sem metadado embutido, a restauração perde a localização.

## Decisão

O passo de normalização grava os metadados do sidecar dentro de uma cópia do arquivo, em
`derived/normalized/`. O original permanece intocado (ADR-004).

## Consequências

**Mapeamento:** `photoTakenTime` → `EXIF:DateTimeOriginal`; `geoData` → `EXIF:GPS*`
com os campos `Ref` de hemisfério; `description` → `XMP:Description` e
`IPTC:Caption-Abstract`; `people[]` → `XMP-mwg-rs:RegionName` e `XMP:Subject`;
`favorited` → `XMP:Rating = 5`.

**Três propriedades exigidas:** é reprodutível (apagar `derived/` e regerar dá o mesmo
resultado); é verificada por releitura (bibliotecas de EXIF falham em silêncio com frequência);
e é a moeda de troca — é esse arquivo que sobe para o Google e que abre no Lightroom com a
localização certa.

**Armadilha conhecida:** sem `GPSLatitudeRef`/`GPSLongitudeRef`, latitude negativa vira
positiva e a foto do Brasil aparece na Ucrânia.

**Estratégia de implementação:** crates Rust para o caminho comum, ExifTool como backend
**opcional** detectado em runtime. Nunca dependência obrigatória. Para vídeo, grava-se o que o
contêiner aceita e o restante fica só no catálogo — sem fingir paridade com imagens.
