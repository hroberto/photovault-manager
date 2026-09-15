//! Integração com as APIs do Google Fotos, como elas são em 2026.
//!
//! O que existe e o que não existe está registrado no ADR-006. Em resumo: não há como listar a
//! biblioteca, não há como apagar, e o download por API vem sem GPS. O que **há** é o caminho de
//! volta — `photoslibrary.appendonly` envia arquivos e cria álbuns, e o Google lê o EXIF dos
//! bytes recebidos.
//!
//! Este crate implementa esse caminho, mais o controle de cota que uma restauração de seis dias
//! exige.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod auth;
pub mod client;
pub mod quota;

pub use auth::{EntropyError, PkceChallenge, TokenSet, TokenStore};
pub use client::{ApiError, CreatedItem, PendingItem, PhotosClient, BATCH_LIMIT};
pub use quota::{BudgetDecision, DailyBudget};
