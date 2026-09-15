# ADR-012: Advisor, não Executor

**Estado:** Aceito
**Data:** 2026-09-14

## Contexto

A API do Google Fotos não oferece nenhum mecanismo para apagar itens da biblioteca do usuário.
A Library API só gerencia o que o próprio aplicativo enviou. Vender 'expurgo' como pilar do
produto seria prometer o que não se pode entregar.

## Decisão

O PhotoVault analisa, mede, classifica risco, prova que existe backup, gera a lista e leva o
usuário até o item no Google Fotos pelo `google_url` do sidecar. **Ele não apaga nada na conta
do usuário.** O motor de análise fica separado de um eventual executor.

## Consequências

**A favor:** o motor mantém valor integral mesmo que a Google nunca abra a API. A separação
sobrevive a mudanças de plataforma.

**Contra:** o passo final é manual. O usuário apaga em lote pela interface da Google.

**O que o PhotoVault registra:** a intenção, a janela de segurança e o resultado, no log de
auditoria encadeado — de modo que exista histórico do que foi recomendado e quando.
