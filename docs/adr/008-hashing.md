# ADR-008: BLAKE3 como identidade, SHA-256 sob demanda

**Estado:** Aceito
**Data:** 2026-09-14

## Contexto

O projeto precisa de uma função de hash para identificar objetos no CAS e para verificar
integridade. A proposta inicial calculava BLAKE3 e SHA-256 sempre, com o argumento de
'interoperabilidade e validação futura'.

## Decisão

BLAKE3 é a identidade interna do CAS. SHA-256 é calculado sob demanda, apenas quando o acervo
sai do PhotoVault — manifestos de exportação e verificação por ferramentas externas.

## Consequências

**Por que o argumento original não se sustenta:** validação futura contra o quê? O Google não
expõe hash dos seus itens. Não existe referência remota para comparar, então o SHA-256 só ganha
sentido quando alguém de fora precisa conferir o que recebeu.

**A favor:** BLAKE3 é várias vezes mais rápido e paralelizável; calcular um só hash durante a
ingestão economiza tempo real em um acervo de centenas de gigabytes.

**Contra:** um manifesto pedido depois exige reler os objetos. Aceitável — é operação rara.
