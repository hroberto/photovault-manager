# ADR-006: Takeout é canônico, Picker é complementar

**Estado:** Aceito
**Data:** 2026-09-14

## Contexto

Em 2026 há dois caminhos para tirar conteúdo do Google Fotos, e eles não são equivalentes. O
`mediaItems.list` foi removido em 31/03/2025. A Picker API exige seleção manual e — decisivo —
**entrega os bytes sem o bloco GPS do EXIF**. O Takeout entrega os bytes originais com EXIF
intacto e um JSON lateral com geolocalização, pessoas, descrição e favoritos. A Data Portability
API não cobre o Google Fotos.

## Decisão

O Takeout é a fonte canônica de preservação. A Picker API é fonte complementar para o
incremento do dia a dia, sempre marcada como degradada.

## Consequências

**Decorrência direta:** o mesmo item obtido pelas duas vias tem bytes diferentes e, portanto,
hashes diferentes. Sem tratamento isso produz objetos duplicados e falsa contagem de proteção.
Por isso a deduplicação precisa de um hash visual — dos pixels decodificados, ignorando
metadados — e não apenas do hash exato.

**Decorrência de produto:** a captura inicial do acervo é manual e demorada. O assistente do
Takeout mitiga a parte pior coletando os archives pelo Google Drive automaticamente, mas o
acionamento do export continua sendo do usuário. Isso precisa estar claro no onboarding.

**Quando um `ApiDerived` é depois recebido como `Original`, o original o substitui.**
