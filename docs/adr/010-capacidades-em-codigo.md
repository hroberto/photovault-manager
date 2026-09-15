# ADR-010: Capacidades do destino declaradas em código

**Estado:** Aceito
**Data:** 2026-09-14

## Contexto

A tela de restauração precisa dizer ao usuário o que será e o que não será preservado. Escrever
esse texto à mão garante que ele fique desatualizado assim que a Google mudar a API — e um aviso
desatualizado sobre perda de dados é pior que nenhum aviso.

## Decisão

As capacidades de cada destino vivem em uma struct `SinkCapabilities` no domínio. A interface
deriva dela a lista de perdas, a estimativa de dias e os avisos. Nenhum texto de capacidade é
escrito à mão na UI.

## Consequências

**A favor:** quando a Google mudar a API, altera-se uma constante e a tela inteira se corrige.
O teste `google_loses_people_and_favorites_only` trava o comportamento.

**Sutileza que a struct captura:** `explicit_geo = false` mas `reads_exif_geo = true`. O
Google não aceita coordenada como campo de API, mas lê do arquivo. Daí sai
`requires_embedded_geo()`, que torna a normalização obrigatória antes de enviar — uma regra
que seria fácil esquecer e que custaria a geolocalização de todo o acervo restaurado.

**Contra:** um pouco mais de cerimônia para adicionar um destino novo.
