use crate::repository::RepositoryError;

#[derive(Debug, thiserror::Error)]
pub enum ApplicationError {
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<RepositoryError> for ApplicationError {
    fn from(e: RepositoryError) -> Self {
        match e {
            RepositoryError::NotFound => ApplicationError::NotFound("resource not found".into()),
            RepositoryError::VersionConflict => ApplicationError::Conflict(
                "version mismatch: the resource was modified; reload and retry".into(),
            ),
            other => ApplicationError::Internal(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_errors_map_to_proper_application_errors() {
        assert!(matches!(
            ApplicationError::from(RepositoryError::NotFound),
            ApplicationError::NotFound(_)
        ));
        assert!(matches!(
            ApplicationError::from(RepositoryError::VersionConflict),
            ApplicationError::Conflict(_)
        ));
        assert!(matches!(
            ApplicationError::from(RepositoryError::Serde(
                serde_json::from_str::<u8>("x").unwrap_err()
            )),
            ApplicationError::Internal(_)
        ));
    }
}
