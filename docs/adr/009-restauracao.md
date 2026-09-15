# ADR-009: Restauração como recurso de primeira classe

**Estado:** Aceito
**Data:** 2026-09-14

## Contexto

A assimetria descoberta na pesquisa: sair do Google é manual e difícil, voltar é programático e
fácil. Com `photoslibrary.appendonly` é possível enviar arquivos, criar álbuns e definir
descrições — e o Google **lê o EXIF dos bytes recebidos**, de modo que data de captura e
geolocalização são restauradas.

## Decisão

A restauração não é um recurso acessório: é o teste do produto inteiro. O domínio ganha a porta
`MediaSink`, simétrica a `MediaSource`, e uma restauração de amostra periódica alimenta o
indicador de saúde do cofre.

## Consequências

**A favor:** migração entre contas, recuperação de desastre, reidratação seletiva, correção em
massa e saída para outros destinos passam a ser o mesmo mecanismo.

**O princípio:** um backup que nunca foi restaurado não é um backup, é uma esperança.

**Limites que precisam aparecer na interface:** nomes de pessoas e favoritos não voltam (não há
API); o app só adiciona itens a álbuns que ele mesmo criou; a cota de 10.000 requisições por dia
faz uma restauração de 52 mil itens levar cerca de seis dias; e reenviar consome a cota de
armazenamento do Google novamente.
