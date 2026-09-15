//! Orçamento diário de requisições.
//!
//! A cota do Google é de 10.000 requisições por dia, por projeto. Uma restauração de 52 mil
//! itens gasta cerca de 55.600 — quase seis dias. Bater no limite e receber 429 não é acidente,
//! é o curso normal do trabalho.
//!
//! Por isso o orçamento é contado localmente e antecipadamente. A diferença para o usuário é
//! entre ler "cota esgotada, retoma em 7h" e ver o programa falhar com um erro de rede.

use time::{Duration, OffsetDateTime, Time, UtcOffset};

/// Fuso em que a Google zera a cota diária: horário do Pacífico.
///
/// Usamos -8 fixo em vez de seguir o horário de verão. O erro de uma hora, duas vezes por ano,
/// apenas adia a retomada — nunca faz gastar requisição a mais.
const QUOTA_OFFSET: UtcOffset = match UtcOffset::from_hms(-8, 0, 0) {
    Ok(offset) => offset,
    Err(_) => UtcOffset::UTC,
};

/// Controle do que ainda se pode gastar hoje.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyBudget {
    limit: u32,
    spent: u32,
    day: String,
}

/// O que fazer diante de um pedido de requisições.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetDecision {
    /// Pode gastar agora.
    Proceed,
    /// A cota do dia acabou. Retomar depois do instante indicado.
    Exhausted {
        /// Quando a cota zera.
        resumes_at: OffsetDateTime,
        /// Quanto falta esperar.
        wait: Duration,
    },
}

impl BudgetDecision {
    /// Se a operação pode seguir.
    pub const fn is_proceed(&self) -> bool {
        matches!(self, Self::Proceed)
    }

    /// Texto para a interface, em vez de um erro cru.
    pub fn describe(&self) -> String {
        match self {
            Self::Proceed => "dentro da cota".into(),
            Self::Exhausted { wait, .. } => {
                let hours = wait.whole_hours();
                if hours >= 1 {
                    format!("cota do dia esgotada, retoma em {hours}h")
                } else {
                    format!(
                        "cota do dia esgotada, retoma em {}min",
                        wait.whole_minutes().max(1)
                    )
                }
            }
        }
    }
}

impl DailyBudget {
    /// Cria um orçamento para o dia de um instante.
    pub fn new(limit: u32, now: OffsetDateTime) -> Self {
        Self {
            limit,
            spent: 0,
            day: quota_day(now),
        }
    }

    /// Restaura o estado persistido.
    pub fn restore(limit: u32, spent: u32, day: impl Into<String>) -> Self {
        Self {
            limit,
            spent: spent.min(limit),
            day: day.into(),
        }
    }

    /// Dia da cota a que este orçamento se refere.
    pub fn day(&self) -> &str {
        &self.day
    }

    /// Quanto já se gastou hoje.
    pub const fn spent(&self) -> u32 {
        self.spent
    }

    /// Quanto ainda cabe hoje.
    pub const fn remaining(&self) -> u32 {
        self.limit.saturating_sub(self.spent)
    }

    /// Consulta se cabem `requests` requisições agora, virando o dia se for o caso.
    pub fn check(&mut self, requests: u32, now: OffsetDateTime) -> BudgetDecision {
        self.roll_over(now);

        if requests <= self.remaining() {
            return BudgetDecision::Proceed;
        }

        let resumes_at = next_reset(now);
        BudgetDecision::Exhausted {
            resumes_at,
            wait: resumes_at - now,
        }
    }

    /// Registra requisições efetivamente gastas.
    pub fn spend(&mut self, requests: u32, now: OffsetDateTime) {
        self.roll_over(now);
        self.spent = self.spent.saturating_add(requests).min(self.limit);
    }

    /// Zera a contagem quando o dia da cota virou.
    fn roll_over(&mut self, now: OffsetDateTime) {
        let today = quota_day(now);
        if today != self.day {
            self.day = today;
            self.spent = 0;
        }
    }

    /// Quantos dias inteiros de cota um trabalho consome.
    ///
    /// É esta a estimativa que aparece na tela antes de a restauração começar — em dias, não
    /// numa porcentagem inventada.
    pub fn days_for(&self, total_requests: u64) -> u64 {
        if self.limit == 0 {
            return 0;
        }
        total_requests.div_ceil(u64::from(self.limit))
    }
}

/// Dia da cota, no fuso em que a Google a zera.
fn quota_day(now: OffsetDateTime) -> String {
    let local = now.to_offset(QUOTA_OFFSET);
    format!(
        "{:04}-{:02}-{:02}",
        local.year(),
        u8::from(local.month()),
        local.day()
    )
}

/// Próxima meia-noite no fuso da cota.
fn next_reset(now: OffsetDateTime) -> OffsetDateTime {
    let local = now.to_offset(QUOTA_OFFSET);
    let midnight = local
        .replace_time(Time::MIDNIGHT)
        .saturating_add(Duration::days(1));
    midnight.to_offset(UtcOffset::UTC)
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn spends_within_the_limit() {
        let now = datetime!(2026-09-14 12:00:00 UTC);
        let mut budget = DailyBudget::new(10_000, now);

        assert_eq!(budget.remaining(), 10_000);
        assert!(budget.check(50, now).is_proceed());
        budget.spend(50, now);
        assert_eq!(budget.spent(), 50);
        assert_eq!(budget.remaining(), 9_950);
    }

    #[test]
    fn refuses_when_the_request_does_not_fit() {
        let now = datetime!(2026-09-14 12:00:00 UTC);
        let mut budget = DailyBudget::new(100, now);
        budget.spend(99, now);

        assert!(budget.check(1, now).is_proceed());
        let decision = budget.check(2, now);
        assert!(!decision.is_proceed());
        assert!(matches!(decision, BudgetDecision::Exhausted { .. }));
    }

    #[test]
    fn exhaustion_says_when_it_resumes() {
        // 12:00 UTC é 04:00 no fuso da cota; faltam 20 horas para a virada.
        let now = datetime!(2026-09-14 12:00:00 UTC);
        let mut budget = DailyBudget::new(10, now);
        budget.spend(10, now);

        let decision = budget.check(1, now);
        match &decision {
            BudgetDecision::Exhausted { wait, resumes_at } => {
                assert_eq!(wait.whole_hours(), 20);
                assert!(*resumes_at > now);
            }
            other => panic!("esperava esgotamento, veio {other:?}"),
        }
        // A interface mostra isto, não um erro de rede.
        assert_eq!(decision.describe(), "cota do dia esgotada, retoma em 20h");
    }

    #[test]
    fn minutes_when_the_reset_is_close() {
        // 07:30 UTC do dia seguinte é 23:30 do dia anterior no fuso da cota.
        let now = datetime!(2026-09-15 07:30:00 UTC);
        let mut budget = DailyBudget::new(1, now);
        budget.spend(1, now);

        assert_eq!(
            budget.check(1, now).describe(),
            "cota do dia esgotada, retoma em 30min"
        );
    }

    #[test]
    fn rolls_over_at_the_quota_midnight() {
        let before = datetime!(2026-09-15 07:59:00 UTC); // 23:59 no fuso da cota
        let after = datetime!(2026-09-15 08:01:00 UTC); // 00:01 do dia seguinte

        let mut budget = DailyBudget::new(100, before);
        budget.spend(100, before);
        assert_eq!(budget.remaining(), 0);

        // Passou da meia-noite da cota: o orçamento zera sozinho.
        assert!(budget.check(50, after).is_proceed());
        assert_eq!(budget.spent(), 0);
        assert_ne!(budget.day(), quota_day(before));
    }

    #[test]
    fn quota_day_follows_the_pacific_offset() {
        // 07:00 UTC ainda é o dia anterior no fuso da cota.
        assert_eq!(quota_day(datetime!(2026-09-15 07:00:00 UTC)), "2026-09-14");
        assert_eq!(quota_day(datetime!(2026-09-15 09:00:00 UTC)), "2026-09-15");
    }

    #[test]
    fn restores_persisted_state() {
        let budget = DailyBudget::restore(10_000, 7_432, "2026-09-14");
        assert_eq!(budget.spent(), 7_432);
        assert_eq!(budget.remaining(), 2_568);
        assert_eq!(budget.day(), "2026-09-14");
    }

    #[test]
    fn restore_clamps_impossible_values() {
        // Um banco corrompido não pode produzir crédito negativo nem estouro.
        let budget = DailyBudget::restore(100, 5_000, "2026-09-14");
        assert_eq!(budget.remaining(), 0);
    }

    #[test]
    fn spending_never_overflows() {
        let now = datetime!(2026-09-14 12:00:00 UTC);
        let mut budget = DailyBudget::new(10, now);
        budget.spend(u32::MAX, now);
        assert_eq!(budget.remaining(), 0);
        assert_eq!(budget.spent(), 10);
    }

    #[test]
    fn full_library_restore_takes_six_days() {
        // Os números do acervo de referência: 52.152 itens, ~55.600 requisições.
        let budget = DailyBudget::new(10_000, datetime!(2026-09-14 12:00:00 UTC));
        assert_eq!(budget.days_for(55_600), 6);
        assert_eq!(budget.days_for(1_310), 1, "um álbum termina no mesmo dia");
        assert_eq!(budget.days_for(0), 0);
    }
}
