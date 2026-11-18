//! Detector de ausencia: ha quanto tempo ninguem mexe no mouse/teclado (`GetLastInputInfo`).
//! Base para adiantar trabalho pesado com o PC livre; por enquanto so informa (nada depende dele).

/// Sem mouse/teclado por mais que isso = usuario ausente.
pub const AWAY_SECS: u64 = 120;

/// Segundos desde a ultima entrada do usuario nesta sessao do Windows. `None` fora do Windows ou se a API falhar.
pub fn idle_secs() -> Option<u64> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::GetTickCount;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
        let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        // SAFETY: estrutura inicializada com cbSize correto; a API so escreve nela.
        if unsafe { GetLastInputInfo(&mut info) } == 0 {
            return None;
        }
        // GetTickCount volta a zero a cada ~49 dias: a subtracao com wrap continua certa
        let now = unsafe { GetTickCount() };
        Some(u64::from(now.wrapping_sub(info.dwTime)) / 1000)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Usuario ausente (sem entrada ha mais de `AWAY_SECS`). Sem como saber, assume presente (o lado seguro).
pub fn is_away() -> bool {
    away(idle_secs())
}

fn away(idle: Option<u64>) -> bool {
    idle.is_some_and(|s| s > AWAY_SECS)
}

/// Texto curto para status/log.
pub fn describe() -> String {
    match idle_secs() {
        Some(s) if away(Some(s)) => format!("ausente (sem uso ha {} min)", s / 60),
        Some(_) => "em uso".into(),
        None => "desconhecido".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn away_limit() {
        assert!(!away(None));
        assert!(!away(Some(0)));
        assert!(!away(Some(AWAY_SECS)));
        assert!(away(Some(AWAY_SECS + 1)));
    }

    #[cfg(windows)]
    #[test]
    fn api_answers() {
        assert!(idle_secs().is_some());
    }
}
