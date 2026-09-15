# ADR-004: CAS com objetos imutáveis

**Estado:** Aceito
**Data:** 2026-09-14

## Contexto

A mesma foto aparece na linha do tempo e em cada álbum do Takeout. Guardá-la N vezes desperdiça
espaço e torna a deduplicação um problema posterior. Ao mesmo tempo, o projeto precisa reescrever
metadados nos arquivos (ADR-013) — e escrever no objeto mudaria seu hash, destruindo a
identidade que o nomeia.

## Decisão

Armazenamento endereçado por conteúdo: o nome do arquivo é o BLAKE3 dos seus bytes, com fanout
de dois caracteres. **Objetos nunca são modificados.** A normalização produz cópias em
`derived/normalized/`, fora do repositório canônico.

## Consequências

**A favor:** deduplicação exata é consequência do armazenamento, não uma varredura; integridade
verificável a qualquer momento; `repository/` pode ser copiado sozinho, e `derived/` pode ser
apagado sem perda.

**Como é imposto:** objetos são gravados em modo `0444`. Abrir um deles para escrita falha no
sistema de arquivos. A regra deixou de ser documentação e virou comportamento — ver o teste
`stored_objects_are_immutable` em `crates/cas`.

**Contra:** nomes de arquivo ilegíveis para um humano navegando no disco. Aceitável: a leitura
humana é papel do catálogo e da futura exportação para árvore `ano/mês`.
