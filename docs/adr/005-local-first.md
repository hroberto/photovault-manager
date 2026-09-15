# ADR-005: Local-first, sem servidor

**Estado:** Aceito
**Data:** 2026-09-14

## Contexto

O acervo é o material mais pessoal que uma pessoa tem. Qualquer arquitetura que faça esses
dados passarem por um servidor nosso cria risco de privacidade, custo recorrente e um ponto de
falha que sobrevive ao interesse de quem escreveu o software.

## Decisão

Todo o processamento acontece na máquina do usuário. Não existe servidor do PhotoVault. A
internet é usada apenas para OAuth do Google, chamadas às APIs do Google, downloads e
atualizações.

## Consequências

**A favor:** privacidade por construção; nenhum custo de operação; funciona offline para tudo
que não seja sincronizar; o software continua útil mesmo que o projeto seja abandonado.

**Contra:** nada de sincronização entre dispositivos do próprio usuário sem ele montar isso
(NAS, pasta sincronizada). Reconhecimento facial e busca semântica precisam rodar localmente,
o que limita os modelos utilizáveis.

**Compromisso firme:** dados de rosto e embeddings nunca saem da máquina.
