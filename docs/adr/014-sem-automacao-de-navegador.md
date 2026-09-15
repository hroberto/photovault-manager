# ADR-014: Nenhuma automação de navegador

**Estado:** Aceito
**Data:** 2026-09-14

## Contexto

Diante da ausência de API de deleção (ADR-012), a tentação é automatizar cliques na interface
web do Google Fotos com Selenium ou Playwright para apagar fotos em lote.

## Decisão

O PhotoVault não automatiza navegador para nenhuma operação, em nenhuma circunstância.

## Consequências

**Por quê:** é frágil — qualquer mudança de layout quebra tudo, e a quebra pode acontecer no
meio de uma operação destrutiva; viola os termos de serviço do Google e arrisca a conta do
usuário; exige manipular a sessão autenticada fora do fluxo OAuth; e um seletor que mudou de
significado pode apagar a coisa errada.

**O que se faz no lugar:** o Advisor gera a lista com `google_url` de cada item, que abre a
foto exata no Google Fotos. O usuário apaga em lote pela interface deles, com o controle e a
responsabilidade do lado certo.

**Isto não é negociável** nem sob pedido do usuário: o modo de falha é a perda silenciosa de
dados irrepetíveis.
