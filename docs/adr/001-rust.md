# ADR-001: Rust como linguagem principal

**Estado:** Aceito
**Data:** 2026-09-14

## Contexto

O software manipula dados irrepetíveis: as fotos de uma vida. Um erro de memória ou uma
condição de corrida que corrompa um arquivo não tem desfazer. O trabalho é dominado por I/O e
hashing de dezenas de milhares de arquivos, com jobs que rodam por dias.

## Decisão

Rust para todo o núcleo: domínio, ingestão, armazenamento, catálogo e restauração.

## Consequências

**A favor:** segurança de memória sem coletor de lixo; concorrência com garantias do
compilador; binário nativo de baixo consumo; `Result` força o tratamento de erro onde ele
importa.

**Contra:** o ecossistema é fraco em três pontos deste projeto — escrita de EXIF, hash
perceptual e embeddings. A mitigação está no ADR-013 (ExifTool opcional) e no adiamento dos
outros dois para depois da V1.

**Custo aceito:** compilação mais lenta e curva de aprendizado maior que a de Go ou Python.
