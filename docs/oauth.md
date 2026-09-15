# Autenticação via OAuth com o Google

[Documentação](README.md) · [README do projeto](../README.md)

**Verificação: 15/09/2026.** Este guia cobre a configuração externa e o fluxo previsto.
**A CLI ainda não oferece login OAuth nem restauração.** A importação local do Takeout
funciona sem essas credenciais.

## Sumário

- [Estado da implementação](#estado-da-implementação)
- [Configuração no Google Cloud](#configuração-no-google-cloud)
- [Escopos](#escopos)
- [Fluxo previsto](#fluxo-previsto)
- [Expiração e problemas comuns](#expiração-e-problemas-comuns)

## Estado da implementação

| Etapa | Estado no código |
| --- | --- |
| Gerar PKCE e URL de autorização | Implementado em `crates/google/src/auth.rs` |
| Representar tokens e conferir escopos concedidos | Implementado em `TokenSet` |
| Gerar e validar `state`, abrir navegador e receber retorno local | Pendente; a função de URL apenas recebe `state` e `redirect_uri` |
| Trocar código por tokens e renovar acesso | Pendente; existe apenas a constante do endpoint |
| Persistir credenciais no chaveiro do sistema | Pendente; `TokenStore` só tem implementação em memória para testes |
| Autenticar e executar restauração pela CLI | Pendente |

Fontes locais: [módulo OAuth](../crates/google/src/auth.rs),
[cliente Google](../crates/google/src/client.rs) e [comandos da CLI](../crates/cli/src/main.rs).

Não há opção `--client-id`, leitura de JSON de credenciais ou variável de ambiente OAuth
implementada. Criar o cliente abaixo prepara a integração futura; não conclui a autenticação.

## Configuração no Google Cloud

### 1. Criar o projeto e ativar a API

1. Acesse o [Google Cloud Console](https://console.cloud.google.com/) com uma conta que possa
   criar ou administrar o projeto.
2. Crie ou selecione um projeto, por exemplo, `PhotoVault Manager`.
3. Em **APIs e serviços → Biblioteca**, ative **Photos Library API** para a restauração.

**Photos Picker API** só será necessária para a futura seleção de fotos existentes.
Ela é diferente de **Google Picker API**. As APIs do Google Fotos exigem autorização de uma
conta de usuário; não aceitam contas de serviço. Veja a
[configuração oficial das APIs](https://developers.google.com/photos/overview/configure-your-app).

### 2. Configurar a tela de consentimento

1. Abra **Google Auth Platform → Branding** e inicie a configuração, se necessário.
2. Preencha nome do aplicativo, e-mail de suporte e contato.
3. Em **Audience**, escolha **External** para uma conta pessoal. **Internal** se aplica a
   usuários da organização Google Workspace.
4. Para desenvolvimento, mantenha **Testing** e adicione em **Test users** a conta do Google
   Fotos que autorizará o acesso. Ela pode ser diferente da conta que administra o projeto.
5. Em **Data Access → Add or Remove Scopes**, adicione os dois escopos de restauração da
   próxima seção e salve.

Os nomes dos menus podem aparecer traduzidos. Consulte a
[configuração oficial do consentimento](https://developers.google.com/workspace/guides/configure-oauth-consent).

### 3. Criar o Client ID

Em **Google Auth Platform → Clients → Create client**, escolha **Desktop app**
(aplicativo para computador), dê um nome e copie o Client ID. Guarde eventual JSON baixado
fora do repositório. A CLI ainda não o lê.

O retorno previsto usa `http://127.0.0.1:<porta>`. A documentação trata `client_secret` como
opcional nesse fluxo; PKCE não transforma um aplicativo desktop em cliente confidencial.
Veja [OAuth para aplicativos instalados](https://developers.google.com/identity/protocols/oauth2/native-app).

Mantenha o Client ID usado nas restaurações: o acesso aos recursos criados depende do cliente
original, conforme a [orientação sobre troca de Client ID](https://developers.google.com/photos/overview/configure-your-app#changing-your-client-id).

## Escopos

Um escopo define a permissão solicitada à conta. Para enviar e verificar os itens restaurados,
o projeto prevê estes valores completos:

| Finalidade | Escopo |
| --- | --- |
| Enviar mídia e criar álbuns | `https://www.googleapis.com/auth/photoslibrary.appendonly` |
| Reler mídia e álbuns criados pelo aplicativo | `https://www.googleapis.com/auth/photoslibrary.readonly.appcreateddata` |

O futuro Picker usa `https://www.googleapis.com/auth/photospicker.mediaitems.readonly`.
O módulo também declara `SCOPE_DRIVE_READONLY` para a futura coleta de arquivos do Takeout
no Drive, mas não implementa esse cliente. Esses recursos não são pré-requisitos da restauração.

Editar itens ou reorganizar álbuns já criados pelo aplicativo pode exigir
`photoslibrary.edit.appcreateddata`, que ainda não está declarado no módulo OAuth.
Os escopos antigos de leitura geral não recuperam o acesso a todo o acervo. Consulte os
[escopos oficiais](https://developers.google.com/photos/overview/authorization).

## Fluxo previsto

Esta sequência orienta a implementação pendente:

1. Abrir um receptor HTTP apenas no loopback, em porta livre; gerar PKCE e `state` aleatório.
2. Usar `authorization_url` e abrir o navegador do sistema. A função já inclui `S256`,
   `access_type=offline` e `prompt=consent`.
3. Receber `code` ou erro, validar `state` e encerrar o receptor.
4. Enviar por HTTPS ao endpoint de tokens: `client_id`, `code`, `code_verifier`,
   `redirect_uri` idêntico e `grant_type=authorization_code`.
5. Conferir permissões; manter o acesso em memória e guardar a renovação no chaveiro.
6. Renovar com `grant_type=refresh_token` quando necessário, preservando o token de renovação
   anterior se a resposta não trouxer outro.

O verificador PKCE fica fora da URL e é enviado ao Google na troca do código.
Referência: [fluxo OAuth desktop](https://developers.google.com/identity/protocols/oauth2/native-app).
O armazenamento seguro segue o [modelo de segurança do projeto](../SECURITY.md).

## Expiração e problemas comuns

Em projetos **External / Testing**, os escopos de Fotos fazem o `refresh_token` expirar em
**sete dias**. Uma restauração longa precisa tratar nova autorização e retomada. Publicar o app
não torna tokens permanentes: revogação e outros limites continuam aplicáveis. Veja a
[expiração de tokens](https://developers.google.com/identity/protocols/oauth2#expiration).

| Sintoma | O que conferir |
| --- | --- |
| `photovault auth` ou `photovault restore` não reconhecido | Esses comandos ainda não existem; confirme com `--help` |
| Conta de teste sem acesso | Conta adicionada em **Audience → Test users** e projeto correto |
| `redirect_uri_mismatch` | Tipo Desktop e URI de retorno consistente |
| `invalid_grant` durante a renovação | Expiração ou revogação; pode ser necessária nova autorização |
| Permissão insuficiente ao chamar a API | API habilitada e escopos efetivamente concedidos |
| Retorno local sem conexão | Receptor pendente na CLI atual; futuramente, navegador e receptor devem alcançar o mesmo loopback |

Para erros do provedor, consulte o [fluxo oficial e seus erros](https://developers.google.com/identity/protocols/oauth2/native-app)
e as [regras de renovação](https://developers.google.com/identity/protocols/oauth2#expiration).
Não registre tokens, códigos de autorização ou verificadores PKCE em logs ou issues.
