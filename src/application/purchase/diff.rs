use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct FieldChange<T: Serialize> {
    pub from: T,
    pub to: T,
}

#[derive(Debug, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum PurchaseChanges {
    PurchaseUpdated { fields: PurchaseFieldChanges },
    PurchaseDeleted,
    ItemAdded { items: Vec<ItemSnapshot> },
    ItemUpdated { items: Vec<ItemDiff> },
    ItemRemoved { items: Vec<ItemSnapshot> },
}

#[derive(Debug, Default, Serialize)]
pub struct PurchaseFieldChanges {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<FieldChange<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<FieldChange<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<FieldChange<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_id: Option<FieldChange<i32>>,
}

#[derive(Debug, Serialize)]
pub struct ItemSnapshot {
    pub item_sequence: i32,
    pub name: String,
    pub unit_price: f64,
    pub quantity: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ItemDiff {
    pub item_sequence: i32,
    pub from: ItemSnapshot,
    pub to: ItemSnapshot,
}
