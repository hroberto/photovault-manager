# ADR-007: Fidelidade como propriedade de primeira classe

**Estado:** Aceito
**Data:** 2026-09-14

## Contexto

Da assimetria do ADR-006 decorre que nem todo byte que representa uma foto vale o mesmo. Tratar
uma cópia sem GPS como equivalente ao original corrompe a deduplicação, mente no indicador de
proteção e — o pior — pode sustentar uma recomendação de apagar o original.

## Decisão

Todo objeto carrega uma `Fidelity`: `Original`, `ApiDerived`, `Normalized` ou
`Derivative`. É um tipo do domínio, não um campo booleano nem uma string.

## Consequências

**A favor:** a regra deixa de depender de alguém lembrar. `Fidelity::is_canonical()` é
consultada pelo motor de expurgo, e `CleanupEligibility::blocker()` devolve **o motivo** da
recusa, não um booleano — porque a interface precisa explicar ao usuário por que aquele item não
pode ser eliminado.

**Regra derivada:** apenas `Original` sustenta expurgo. `ApiDerived` é provisório.
`Normalized` é reprodutível e descartável.

**Contra:** mais um conceito para o usuário entender. Mitigado exibindo-o só onde muda a
decisão.
