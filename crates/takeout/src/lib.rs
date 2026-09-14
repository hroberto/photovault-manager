//! Leitura de archives do Google Takeout.
//!
//! Componente de maior risco técnico do PhotoVault. O Takeout não tem especificação, o formato
//! muda sem aviso, e os dados que ele carrega são irrepetíveis. Por isso, duas regras:
//!
//! - Nada é adivinhado em silêncio. Uma associação incerta vira órfão com motivo registrado.
//! - Toda variação encontrada em archive real vira fixture antes de virar correção de código.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod filename;
pub mod matcher;
pub mod sidecar;

pub use filename::MediaKey;
pub use matcher::{
    match_directory, Match, MatchConfidence, MatchReport, OrphanReason, OrphanSidecar,
};
pub use sidecar::{Sidecar, SidecarError};
