//! Domínio do PhotoVault.
//!
//! Este crate não conhece o Google, nem SQLite, nem sistema de arquivos. Ele define o que um
//! item de mídia é, de onde veio, quanta fidelidade tem e o que pode ser feito com ele.
//!
//! Regra: se uma struct daqui precisa de um campo específico de um provedor, ela está no
//! lugar errado.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod fidelity;
pub mod ids;
pub mod media;
pub mod person;
pub mod place;
pub mod sink;

pub use fidelity::{Fidelity, VerificationState};
pub use ids::{MediaId, ObjectHash, RemoteMediaId};
pub use media::{MediaItem, MediaKind, SourceKind};
pub use person::PersonName;
pub use place::GeoPoint;
pub use sink::SinkCapabilities;
