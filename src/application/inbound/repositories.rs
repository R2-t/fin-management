#![allow(async_fn_in_trait)]

use crate::application::error::ApplicationError;
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
    pub id: uuid::Uuid,
    pub store: Option<String>,
    pub description: Option<String>,
    pub notes: Option<String>,
    pub category_id: Option<i32>,
}

pub trait PurchaseRepository {
    async fn create(
        &self,
        wallet_id: uuid::Uuid,
        store: Option<String>,
        notes: Option<String>,
        user_id: uuid::Uuid,
        category_id: i32,
        purchase_date: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Purchase>;
    async fn get(&self, id: uuid::Uuid, user_id: uuid::Uuid) -> Result<Purchase>;
    async fn list(&self, user_id: uuid::Uuid, filters: PurchaseFilters) -> Result<Vec<Purchase>>;
    async fn update(
        &self,
        user_id: uuid::Uuid,
        current_version: i32,
        purchase_update: PurchaseUpdates,
    ) -> Result<Purchase>;
    async fn delete(&self, id: uuid::Uuid, user_id: uuid::Uuid) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Item
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ItemInput {
    pub name: String,
    pub quantity: i32,
    pub unit_price: f64,
    pub item_type: Option<String>,
}

pub trait ItemRepository {
    async fn create(&self, purchase_id: uuid::Uuid, item: ItemInput) -> Result<Item>;
    async fn bulk_create(
        &self,
        purchase_id: uuid::Uuid,
        items: Vec<ItemInput>,
    ) -> Result<Vec<Item>>;
    async fn update(
        &self,
        purchase_id: uuid::Uuid,
        sequence: i32,
        item: ItemInput,
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
    async fn insert(
        &self,
        purchase_id: uuid::Uuid,
        version: i32,
        changes: serde_json::Value,
    ) -> Result<PurchaseHistory>;
    async fn get_for_purchase(
        &self,
        purchase_id: uuid::Uuid,
    ) -> Result<Vec<PurchaseHistory>>;
}
