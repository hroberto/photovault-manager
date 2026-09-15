# ADR-003: SQLite como catálogo

**Estado:** Aceito
**Data:** 2026-09-14

## Contexto

O catálogo precisa de transações, consultas relacionais e integridade referencial para dezenas
de milhares de itens, em um aplicativo desktop que não pode exigir instalação de servidor.

## Decisão

SQLite acessado por SQLx, com migrações versionadas, tabelas `STRICT`, e os PRAGMAs
`journal_mode=WAL`, `foreign_keys=ON`, `synchronous=NORMAL` e `busy_timeout=5000`.

## Consequências

**A favor:** zero infraestrutura; o cofre inteiro é copiável como arquivos; transações reais;
WAL permite um escritor e vários leitores.

**Contra:** um único escritor. A engine de jobs é desenhada em torno disso — se surgir a
necessidade de um segundo escritor, o desenho está errado.

**Decorrência:** consultas usam `sqlx::query` em tempo de execução, não as macros, para não
exigir banco disponível em tempo de compilação. Perde-se a verificação estática do SQL; ganha-se
build reproduzível sem `DATABASE_URL`.
