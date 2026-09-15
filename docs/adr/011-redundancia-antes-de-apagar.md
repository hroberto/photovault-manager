# ADR-011: Nenhuma eliminação sem redundância verificada

**Estado:** Aceito
**Data:** 2026-09-14

## Contexto

O objetivo final do usuário inclui liberar espaço no Google. É exatamente o momento de maior
risco do produto: sugerir apagar algo que não está realmente salvo é a falha que destrói a
confiança e o acervo.

## Decisão

Um item só é sugerido para eliminação se cumprir as cinco pré-condições: fidelidade
`Original`, estado `Redundant`, existência em ao menos duas mídias, última restauração de
amostra bem-sucedida e janela de segurança vencida.

## Consequências

**Como é imposto:** `CleanupEligibility::blocker()` reúne as cinco em um lugar só e devolve o
motivo do bloqueio. Ninguém precisa lembrar de todas, e a interface tem o que explicar.

**O que `Verified` significa e não significa:** significa que os bytes foram relidos do disco
e o hash bateu — integridade **local**. Não significa 'idêntico ao que está no Google', porque o
Google não expõe hash e a API altera os bytes. A interface precisa dizer exatamente isso; prometer
mais seria mentira.

**O que substitui a comparação com o Google:** a restauração de amostra (ADR-009).
