use crate::{
    application::{error::ApplicationError, inbound::repositories::PurchaseHistoryRepository},
    model::purchase_history::PurchaseHistory,
};
use tracing::instrument;

pub struct PurchaseHistoryService<P>
where
    P: PurchaseHistoryRepository,
{
    purchase_history: P,
}

impl<P> PurchaseHistoryService<P>
where
    P: PurchaseHistoryRepository,
{
    pub fn new(purchase_history: P) -> Self {
        Self { purchase_history }
    }

    #[instrument(skip_all, err(level = "warn"), fields(purchase_id = %purchase_id))]
    pub async fn list_for_purchase(
        &self,
        purchase_id: uuid::Uuid,
    ) -> Result<Vec<PurchaseHistory>, ApplicationError> {
        self.purchase_history.get_for_purchase(purchase_id).await
    }
}
