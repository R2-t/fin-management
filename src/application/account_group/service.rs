use tracing::instrument;

use crate::application::error::ApplicationError;
use crate::application::inbound::repositories::{AccountGroupRepository, Result};
use crate::model::account_group::AccountGroup;

/// Groups wallets for reporting.
///
/// A group is a **filter, not a consolidation**: the app never sums across
/// currencies (ADR-0002), so a group's totals are per currency. That is why this
/// service stores membership and nothing else — it deliberately has no "total".
pub struct AccountGroupService<G>
where
    G: AccountGroupRepository,
{
    groups: G,
}

impl<G> AccountGroupService<G>
where
    G: AccountGroupRepository,
{
    pub fn new(groups: G) -> Self {
        Self { groups }
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id))]
    pub async fn create(&self, user_id: uuid::Uuid, name: String) -> Result<AccountGroup> {
        self.groups.create(user_id, validated_name(name)?).await
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id))]
    pub async fn list(&self, user_id: uuid::Uuid) -> Result<Vec<AccountGroup>> {
        self.groups.list(user_id).await
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, group_id = %id))]
    pub async fn rename(
        &self,
        user_id: uuid::Uuid,
        id: uuid::Uuid,
        name: String,
    ) -> Result<AccountGroup> {
        self.groups.rename(user_id, id, validated_name(name)?).await
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, group_id = %id))]
    pub async fn delete(&self, user_id: uuid::Uuid, id: uuid::Uuid) -> Result<()> {
        self.groups.delete(user_id, id).await
    }

    /// Sets or clears a wallet's group.
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, wallet_id = %wallet_id))]
    pub async fn assign(
        &self,
        user_id: uuid::Uuid,
        wallet_id: uuid::Uuid,
        group_id: Option<uuid::Uuid>,
    ) -> Result<()> {
        self.groups.assign_wallet(user_id, wallet_id, group_id).await
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, group_id = %group_id))]
    pub async fn wallets(&self, user_id: uuid::Uuid, group_id: uuid::Uuid) -> Result<Vec<uuid::Uuid>> {
        self.groups.wallets_in(user_id, group_id).await
    }
}

/// A group name is a label the user reads: non-empty, and not padded, because
/// `" Savings "` and `"Savings"` are the same group.
fn validated_name(name: String) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ApplicationError::Validation(
            "an account group name cannot be empty".into(),
        ));
    }
    if trimmed.chars().count() > 64 {
        return Err(ApplicationError::Validation(
            "an account group name cannot be longer than 64 characters".into(),
        ));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_trimmed() {
        assert_eq!(validated_name("  Savings ".into()).unwrap(), "Savings");
    }

    #[test]
    fn an_empty_name_is_refused() {
        assert!(validated_name("   ".into()).is_err());
    }

    #[test]
    fn an_over_long_name_is_refused() {
        assert!(validated_name("x".repeat(65)).is_err());
        assert!(validated_name("x".repeat(64)).is_ok());
    }
}
