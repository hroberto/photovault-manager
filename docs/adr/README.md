# Registros de Decisão de Arquitetura

Cada ADR registra uma decisão, o contexto em que foi tomada e o que ela custa. São documentos
imutáveis: uma decisão que muda não é editada, é substituída por um ADR novo que a supera.

| # | Decisão | Estado |
| --- | --- | --- |
| [001](001-rust.md) | Rust como linguagem principal | Aceito |
| [002](002-tauri.md) | Tauri 2 para a interface | Aceito |
| [003](003-sqlite.md) | SQLite como catálogo | Aceito |
| [004](004-cas-imutavel.md) | CAS com objetos imutáveis | Aceito |
| [005](005-local-first.md) | Local-first, sem servidor | Aceito |
| [006](006-takeout-canonico.md) | Takeout é canônico, Picker é complementar | Aceito |
| [007](007-fidelidade.md) | Fidelidade como propriedade de primeira classe | Aceito |
| [008](008-hashing.md) | BLAKE3 como identidade, SHA-256 sob demanda | Aceito |
| [009](009-restauracao.md) | Restauração como recurso de primeira classe | Aceito |
| [010](010-capacidades-em-codigo.md) | Capacidades do destino declaradas em código | Aceito |
| [011](011-redundancia-antes-de-apagar.md) | Nenhuma eliminação sem redundância verificada | Aceito |
| [012](012-advisor-nao-executor.md) | Advisor, não Executor | Aceito |
| [013](013-metadados-no-arquivo.md) | Metadados embutidos nos arquivos | Aceito |
| [014](014-sem-automacao-de-navegador.md) | Nenhuma automação de navegador | Aceito |
