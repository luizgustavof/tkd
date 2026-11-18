use serde::Serialize;
use specta::Type;

/// Erro que chega na tela: codigo estavel + mensagem em pt-BR.
#[derive(Debug, Clone, Serialize, Type, thiserror::Error)]
#[error("{message}")]
pub struct AppError {
    /// nao_encontrado | ocupado | invalido | erro
    pub code: String,
    pub message: String,
}

impl AppError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into() }
    }
    pub fn busy(m: impl Into<String>) -> Self {
        Self::new("ocupado", m)
    }
    pub fn invalid(m: impl Into<String>) -> Self {
        Self::new("invalido", m)
    }
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        let message = format!("{e:#}");
        let code = if message.contains("nao existe") {
            "nao_encontrado"
        } else if message.contains("ja tem um video") || message.contains("ja existe") {
            "ocupado"
        } else {
            "erro"
        };
        Self::new(code, message)
    }
}

impl From<tauri::Error> for AppError {
    fn from(e: tauri::Error) -> Self {
        Self::new("erro", e.to_string())
    }
}

impl From<tokio::task::JoinError> for AppError {
    fn from(e: tokio::task::JoinError) -> Self {
        Self::new("erro", format!("tarefa interrompida: {e}"))
    }
}

pub type AppResult<T> = Result<T, AppError>;
