# Segurança

[Índice da documentação](docs/README.md) · [Configuração OAuth](docs/oauth.md)

## Modelo de ameaça

O PhotoVault é um aplicativo local que guarda o acervo pessoal completo de alguém e detém
credenciais da conta Google dessa pessoa. O adversário relevante **não** é um atacante remoto
sofisticado. É, em ordem de probabilidade:

1. O próprio software perdendo dados por um bug.
2. Um backup fora do local parar em um serviço que não deveria lê-lo.
3. Um `refresh_token` vazar em log, mensagem de erro ou relatório de falha.
4. Roubo do computador.
5. Uma dependência comprometida.

As defesas seguem essa ordem. Não adianta proteger contra ameaças exóticas enquanto um token
puder aparecer num log.

## Controles implementados

- **PKCE e URL de autorização.** O módulo OAuth gera o desafio e monta a URL com `state`
  fornecido por quem chama. O fluxo completo ainda não está integrado à CLI.
- **Segredos não aparecem em log.** `PkceChallenge` e `TokenSet` têm `Debug` implementado à mão
  para imprimir `<oculto>`, e há testes que falham se um segredo vazar na saída de depuração.
- **`#![forbid(unsafe_code)]`** em todos os crates.

## Requisitos para as próximas integrações

- **Credenciais nunca em texto aberto.** O catálogo prevê uma referência à credencial.
  `TokenStore` define a interface de armazenamento, mas só existe implementação em memória
  para testes. O chaveiro do sistema e o fallback cifrado com Argon2id ainda estão planejados.
- **OAuth com navegador e loopback.** Faltam o receptor local, a geração e validação de `state`,
  a troca do código e a renovação de tokens. O fluxo deve usar o navegador do sistema, conforme
  o [guia OAuth](docs/oauth.md).
- **Entrada não confiável.** A CLI recebe o Takeout já extraído. Um futuro extrator integrado
  deve rejeitar caminhos com `../` (*zip slip*) e limitar a expansão (*zip bomb*).

Os controles planejados acima não devem ser tratados como proteção já disponível.

## Auditoria de dependências

`cargo audit` roda no CI a cada push e também **semanalmente por agendamento**. O agendamento é
o que importa: o risco real é uma CVE divulgada contra uma dependência que não mudou, e ela não
seria notada se a auditoria só rodasse quando alguém empurra código.

### Avisos aceitos conscientemente

Registrados em [`.cargo/audit.toml`](.cargo/audit.toml). Cada um é uma decisão avaliada, não um
silenciamento. Qualquer aviso novo continua derrubando o CI.

| Aviso | Crate | Situação | Revisar em |
| --- | --- | --- | --- |
| [RUSTSEC-2026-0195](https://rustsec.org/advisories/RUSTSEC-2026-0195) | quick-xml 0.37.5 | aceito | 2026-12-15 |
| [RUSTSEC-2026-0194](https://rustsec.org/advisories/RUSTSEC-2026-0194) | quick-xml 0.37.5 | aceito | 2026-12-15 |
| [RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071) | rsa 0.9.10 | não se aplica | — |

#### quick-xml — negação de serviço ao analisar XMP

**O que é.** Alocação ilimitada de declarações de namespace e tempo quadrático na checagem de
atributos duplicados. Um XMP malformado dentro de uma imagem pode esgotar a memória ou travar o
processo.

**Como chega até aqui.** `photovault-exif` → `little_exif 0.6.23` → `quick-xml 0.37.5`. O
`little_exif` usa quick-xml para tratar o XMP embutido nas imagens durante a escrita de EXIF.

**Por que não há correção.** A versão corrigida é `quick-xml >= 0.41`. O `little_exif 0.6.23` é
a última publicada e exige `0.37`, que é semver-incompatível. Não existe atualização, e um
`[patch]` não resolve porque `0.42` não satisfaz `^0.37`.

**Por que é aceitável por ora.** É alcançável — o PhotoVault processa arquivos de imagem que
podem ter vindo de terceiros —, mas o impacto é limitado: derruba ou trava o processo de
importação. Não corrompe o cofre, porque os objetos são gravados no CAS **antes** da
normalização, são imutáveis, e a normalização é reprodutível e descartável. O pior caso é o
usuário precisar identificar e separar o arquivo problemático.

**O que muda a decisão.** Uma versão nova do `little_exif` que bumpe o quick-xml. O Dependabot
abre PR automaticamente quando isso acontecer.

**Alternativa se demorar.** Migrar a escrita de metadados para o ExifTool como backend padrão em
vez de opcional, ou substituir o `little_exif` por manipulação direta de segmentos JPEG com
`img-parts`. Ambas são mudanças de porte e não se justificam pelo risco atual.

#### rsa — ataque Marvin

Não se aplica a este projeto. O `rsa` consta do `Cargo.lock` por ser dependência **opcional** do
`sqlx-mysql`, e usamos apenas o backend SQLite (`default-features = false`). Verificado de duas
formas: `cargo tree -i rsa` não o encontra em nenhum grafo de build, e não há artefatos `librsa`
em `target/` após a compilação. O código nunca é compilado nem executado.

O `cargo audit` varre o `Cargo.lock`, que é independente de features — daí o falso positivo.

## Reportar uma vulnerabilidade

Abra uma issue descrevendo o problema. Este é um projeto pessoal sem processo formal de
divulgação coordenada; se o achado for sensível, diga isso na issue sem os detalhes e
combinaremos um canal.
