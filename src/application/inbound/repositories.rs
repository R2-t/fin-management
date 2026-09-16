#![allow(async_fn_in_trait)]

use crate::application::error::ApplicationError;
use crate::application::purchase::diff::PurchaseChanges;
use crate::model::{
    category::Category, item::Item, purchase::Purchase, purchase_history::PurchaseHistory,
    wallet::Wallet,
};

pub type Result<T> = std::result::Result<T, ApplicationError>;

// ---------------------------------------------------------------------------
// Wallet
// ---------------------------------------------------------------------------

pub trait WalletRepository {
    async fn create(&self, user_id: uuid::Uuid, name: String) -> Result<Wallet>;
    async fn list(&self, user_id: uuid::Uuid) -> Result<Vec<Wallet>>;
    async fn get(&self, user_id: uuid::Uuid, id: uuid::Uuid) -> Result<Wallet>;
    async fn update(&self, id: uuid::Uuid, user_id: uuid::Uuid, name: String) -> Result<Wallet>;
    async fn delete(&self, id: uuid::Uuid, user_id: uuid::Uuid) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Purchase
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct PurchaseFilters {
    pub wallet_id: Option<uuid::Uuid>,
    pub category_id: Option<i32>,
    pub store: Option<String>,
    pub start_date: Option<chrono::DateTime<chrono::Utc>>,
    pub end_date: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct PurchaseUpdates {
    pub store: Option<String>,
    pub description: Option<String>,
    pub notes: Option<String>,
    pub category_id: Option<i32>,
}

pub trait PurchaseRepository {
    async fn create(&self, params: CreatePurchaseParams) -> Result<Purchase>;
    async fn create_with_items(
        &self,
        params: CreatePurchaseParams,
        items: Vec<ItemInput>,
    ) -> Result<(Purchase, Vec<Item>)>;
    async fn get(&self, id: uuid::Uuid, user_id: uuid::Uuid) -> Result<Purchase>;
    async fn list(&self, user_id: uuid::Uuid, filters: PurchaseFilters) -> Result<Vec<Purchase>>;
    async fn update(
        &self,
        purchase_id: uuid::Uuid,
        user_id: uuid::Uuid,
        current_version: i32,
        purchase_update: PurchaseUpdates,
        changes: Option<PurchaseChanges>,
    ) -> Result<Purchase>;
    async fn delete(
        &self,
        id: uuid::Uuid,
        user_id: uuid::Uuid,
        changes: PurchaseChanges,
    ) -> Result<()>;
    async fn add_items(
        &self,
        purchase_id: uuid::Uuid,
        user_id: uuid::Uuid,
        current_version: i32,
        items: Vec<ItemInput>,
    ) -> Result<(Purchase, Vec<Item>)>;
    async fn update_item(
        &self,
        purchase_id: uuid::Uuid,
        user_id: uuid::Uuid,
        current_version: i32,
        sequence: i32,
        item: ItemUpdate,
        changes: PurchaseChanges,
    ) -> Result<Purchase>;
    async fn remove_item(
        &self,
        purchase_id: uuid::Uuid,
        user_id: uuid::Uuid,
        current_version: i32,
        sequence: i32,
        changes: PurchaseChanges,
    ) -> Result<Purchase>;
}

/// Parameters required to create a purchase.
#[derive(Clone, Debug)]
pub struct CreatePurchaseParams {
    pub wallet_id: uuid::Uuid,
    pub store: Option<String>,
    pub notes: Option<String>,
    pub user_id: uuid::Uuid,
    pub category_id: i32,
    pub purchase_date: Option<chrono::DateTime<chrono::Utc>>,
    pub currency: String,
    pub exchange_rate: Option<f64>,
    pub rate_source: Option<String>,
    pub rate_at: Option<chrono::DateTime<chrono::Utc>>,
}

// ---------------------------------------------------------------------------
// Item
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ItemInput {
    pub name: String,
    pub quantity: i32,
    pub unit_price: i64,
    pub item_type: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ItemUpdate {
    pub name: Option<String>,
    pub unit_price: Option<i64>,
    pub quantity: Option<i32>,
    pub item_type: Option<String>,
}

pub trait ItemRepository {
    async fn create(&self, purchase_id: uuid::Uuid, item: ItemInput) -> Result<Item>;
    async fn bulk_create(
        &self,
        purchase_id: uuid::Uuid,
        items: Vec<ItemInput>,
    ) -> Result<Vec<Item>>;
    async fn get(&self, purchase_id: uuid::Uuid, sequence: i32) -> Result<Item>;
    async fn update(
        &self,
        purchase_id: uuid::Uuid,
        sequence: i32,
        item: ItemUpdate,
    ) -> Result<Item>;
    async fn delete(&self, purchase_id: uuid::Uuid, sequence: i32) -> Result<()>;
    async fn list_by_purchase(&self, purchase_id: uuid::Uuid) -> Result<Vec<Item>>;
}

// ---------------------------------------------------------------------------
// Category
// ---------------------------------------------------------------------------

pub trait CategoryRepository {
    async fn create(
        &self,
        name: String,
        description: Option<String>,
        user_id: Option<uuid::Uuid>,
        is_system: bool,
    ) -> Result<Category>;
    async fn list(&self, user_id: Option<uuid::Uuid>) -> Result<Vec<Category>>;
    async fn get(&self, id: i32) -> Result<Category>;
}

// ---------------------------------------------------------------------------
// PurchaseHistory
// ---------------------------------------------------------------------------

pub trait PurchaseHistoryRepository {
    async fn get_for_purchase(&self, purchase_id: uuid::Uuid) -> Result<Vec<PurchaseHistory>>;
}

// ---------------------------------------------------------------------------
// Exchange rate cache (top-of-hour, unlogged table)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct CachedRate {
    pub from_currency: String,
    pub to_currency: String,
    pub rate_hour: chrono::DateTime<chrono::Utc>,
    pub rate: f64,
    pub source: String,
    pub stale: bool,
}

pub trait RateCacheRepository {
    async fn get(
        &self,
        from_currency: &str,
        to_currency: &str,
        rate_hour: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<CachedRate>>;
    async fn get_most_recent(
        &self,
        from_currency: &str,
        to_currency: &str,
    ) -> Result<Option<CachedRate>>;
    async fn put(
        &self,
        from_currency: &str,
        to_currency: &str,
        rate_hour: chrono::DateTime<chrono::Utc>,
        rate: f64,
        source: &str,
    ) -> Result<()>;
}
