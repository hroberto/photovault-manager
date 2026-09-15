# ADR-002: Tauri 2 para a interface

**Estado:** Aceito
**Data:** 2026-09-14

## Contexto

A galeria precisa exibir dezenas de milhares de miniaturas com fluidez, e o aplicativo deve
rodar em Windows, macOS e Linux. Electron entregaria isso, mas empacota um Chromium inteiro —
150 MB e centenas de megabytes de RAM ociosa para um software que fica aberto por dias
importando.

## Decisão

Tauri 2 com React e TypeScript, usando a webview do sistema operacional.

## Consequências

**A favor:** binário na casa de 10 MB; consumo de memória muito menor; o núcleo em Rust sem
ponte por serialização pesada; superfície de ataque menor com allowlist explícita.

**Contra:** diferenças entre WebKitGTK, WKWebView e WebView2 exigem teste nas três plataformas.
Algumas APIs de navegador se comportam diferente.

**Decorrência:** miniaturas trafegam por protocolo customizado, nunca como base64 em resposta
de comando — 50 mil miniaturas em base64 destroem a memória do processo.
