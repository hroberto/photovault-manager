//! Fidelidade e proveniência.
//!
//! O conceito central do projeto: nem todo byte que representa uma foto vale o mesmo. Um
//! arquivo vindo do Takeout tem GPS no EXIF; o mesmo item baixado pela API não tem. Tratar os
//! dois como equivalentes corrompe a deduplicação, a contagem de proteção e a decisão de
//! expurgo.

use std::fmt;

/// Quanto se pode confiar nos bytes de um objeto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Fidelity {
    /// Bytes originais com metadados completos. Takeout ou pasta local do usuário.
    ///
    /// É a única fidelidade que sustenta uma recomendação de expurgo.
    Original,
    /// Bytes obtidos por API, com perda conhecida — o Google remove o GPS do EXIF.
    ///
    /// Provisório: quando o mesmo item chegar pelo Takeout, o original o substitui.
    ApiDerived,
    /// Gerado pelo PhotoVault a partir de um `Original`, com metadados reescritos no arquivo.
    ///
    /// Reprodutível e descartável. É o que sobe para o Google numa restauração.
    Normalized,
    /// Derivada de exibição: miniatura, pré-visualização.
    Derivative,
}

impl Fidelity {
    /// Se pode servir de base para eliminar o item da origem.
    ///
    /// Só o original: qualquer outra coisa perdeu metadado no caminho.
    pub const fn is_canonical(self) -> bool {
        matches!(self, Self::Original)
    }

    /// Se deve ser substituída quando uma fidelidade melhor aparecer.
    pub const fn is_provisional(self) -> bool {
        matches!(self, Self::ApiDerived)
    }

    /// Ordem de preferência ao escolher qual cópia de um grupo de duplicatas fica.
    ///
    /// Maior vence.
    pub const fn preference(self) -> u8 {
        match self {
            Self::Original => 3,
            Self::Normalized => 2,
            Self::ApiDerived => 1,
            Self::Derivative => 0,
        }
    }

    /// Rótulo estável para persistência. Nunca mude estes valores sem migração.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::ApiDerived => "api_derived",
            Self::Normalized => "normalized",
            Self::Derivative => "derivative",
        }
    }

    /// Lê o rótulo persistido.
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "original" => Self::Original,
            "api_derived" => Self::ApiDerived,
            "normalized" => Self::Normalized,
            "derivative" => Self::Derivative,
            _ => return None,
        })
    }
}

impl fmt::Display for Fidelity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Onde um objeto está na trajetória entre descoberto e protegido.
///
/// A ordem importa: um estado só avança depois que o anterior foi comprovado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VerificationState {
    /// Sabemos que existe, ainda não temos os bytes.
    Discovered,
    /// Bytes obtidos, ainda em memória ou em arquivo temporário.
    Fetched,
    /// Hash calculado.
    Hashed,
    /// Escrito no CAS.
    Stored,
    /// Relido do disco depois de escrito e o hash bateu.
    ///
    /// Atenção: isto significa integridade **local**. Não significa "idêntico ao que está no
    /// Google" — o Google não expõe hash e a API altera os bytes. Ver `RoadMap.md` seção 23.
    Verified,
    /// Existe em pelo menos duas mídias distintas.
    Redundant,
}

impl VerificationState {
    /// Se o objeto foi comprovado por releitura.
    pub const fn is_verified(self) -> bool {
        matches!(self, Self::Verified | Self::Redundant)
    }

    /// Rótulo estável para persistência.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Discovered => "discovered",
            Self::Fetched => "fetched",
            Self::Hashed => "hashed",
            Self::Stored => "stored",
            Self::Verified => "verified",
            Self::Redundant => "redundant",
        }
    }

    /// Lê o rótulo persistido.
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "discovered" => Self::Discovered,
            "fetched" => Self::Fetched,
            "hashed" => Self::Hashed,
            "stored" => Self::Stored,
            "verified" => Self::Verified,
            "redundant" => Self::Redundant,
            _ => return None,
        })
    }
}

impl fmt::Display for VerificationState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Se um item pode ser sugerido para eliminação na origem.
///
/// As cinco pré-condições do `RoadMap.md` seção 35, reunidas em um lugar só para que ninguém
/// precise lembrar de todas.
#[derive(Debug, Clone, Copy)]
pub struct CleanupEligibility {
    /// Fidelidade dos bytes guardados.
    pub fidelity: Fidelity,
    /// Estado de verificação.
    pub state: VerificationState,
    /// Em quantas mídias distintas o objeto existe.
    pub copies: u8,
    /// Se a última restauração de amostra passou.
    pub sample_restore_passed: bool,
    /// Se a janela de segurança já venceu.
    pub safety_window_elapsed: bool,
}

impl CleanupEligibility {
    /// Motivo pelo qual o item ainda não é elegível, ou `None` se for.
    ///
    /// Devolve o motivo em vez de um booleano porque a interface precisa explicar a recusa.
    pub const fn blocker(&self) -> Option<&'static str> {
        if !self.fidelity.is_canonical() {
            return Some("os bytes guardados não são o original — falta a geolocalização");
        }
        if !matches!(self.state, VerificationState::Redundant) {
            return Some("o objeto ainda não tem cópia redundante verificada");
        }
        if self.copies < 2 {
            return Some("existe em uma única mídia");
        }
        if !self.sample_restore_passed {
            return Some("a última restauração de amostra não passou");
        }
        if !self.safety_window_elapsed {
            return Some("a janela de segurança ainda não venceu");
        }
        None
    }

    /// Se as cinco pré-condições foram atendidas.
    pub const fn is_eligible(&self) -> bool {
        self.blocker().is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eligible() -> CleanupEligibility {
        CleanupEligibility {
            fidelity: Fidelity::Original,
            state: VerificationState::Redundant,
            copies: 3,
            sample_restore_passed: true,
            safety_window_elapsed: true,
        }
    }

    #[test]
    fn only_original_is_canonical() {
        assert!(Fidelity::Original.is_canonical());
        for other in [
            Fidelity::ApiDerived,
            Fidelity::Normalized,
            Fidelity::Derivative,
        ] {
            assert!(!other.is_canonical(), "{other} não pode ser canônica");
        }
    }

    #[test]
    fn original_wins_duplicate_preference() {
        assert!(Fidelity::Original.preference() > Fidelity::Normalized.preference());
        assert!(Fidelity::Normalized.preference() > Fidelity::ApiDerived.preference());
        assert!(Fidelity::ApiDerived.preference() > Fidelity::Derivative.preference());
    }

    #[test]
    fn fidelity_labels_roundtrip() {
        for value in [
            Fidelity::Original,
            Fidelity::ApiDerived,
            Fidelity::Normalized,
            Fidelity::Derivative,
        ] {
            assert_eq!(Fidelity::parse(value.as_str()), Some(value));
        }
    }

    #[test]
    fn state_labels_roundtrip() {
        for value in [
            VerificationState::Discovered,
            VerificationState::Fetched,
            VerificationState::Hashed,
            VerificationState::Stored,
            VerificationState::Verified,
            VerificationState::Redundant,
        ] {
            assert_eq!(VerificationState::parse(value.as_str()), Some(value));
        }
    }

    #[test]
    fn happy_path_is_eligible() {
        assert!(eligible().is_eligible());
    }

    #[test]
    fn api_derived_never_eligible() {
        let mut candidate = eligible();
        candidate.fidelity = Fidelity::ApiDerived;
        assert!(candidate.blocker().is_some());
    }

    #[test]
    fn single_copy_never_eligible() {
        let mut candidate = eligible();
        candidate.state = VerificationState::Verified;
        candidate.copies = 1;
        assert!(candidate.blocker().is_some());
    }

    #[test]
    fn each_precondition_blocks_on_its_own() {
        let mut no_sample = eligible();
        no_sample.sample_restore_passed = false;
        assert!(no_sample.blocker().is_some());

        let mut no_window = eligible();
        no_window.safety_window_elapsed = false;
        assert!(no_window.blocker().is_some());
    }
}
