use serde::Serialize;

use crate::application::inbound::repositories::PurchaseUpdates;
use crate::model::{item::Item, purchase::Purchase};

#[derive(Debug, Clone, Serialize)]
pub struct FieldChange<T: Serialize> {
    pub from: T,
    pub to: T,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum PurchaseChanges {
    PurchaseUpdated { fields: PurchaseFieldChanges },
    PurchaseDeleted,
    ItemAdded { items: Vec<ItemSnapshot> },
    ItemUpdated { items: Vec<ItemDiff> },
    ItemRemoved { items: Vec<ItemSnapshot> },
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct PurchaseFieldChanges {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<FieldChange<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<FieldChange<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<FieldChange<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_id: Option<FieldChange<Option<i32>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merchant_id: Option<FieldChange<Option<uuid::Uuid>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exchange_rate_scaled: Option<FieldChange<Option<i64>>>,
}

impl PurchaseFieldChanges {
    pub fn is_empty(&self) -> bool {
        self.store.is_none()
            && self.description.is_none()
            && self.notes.is_none()
            && self.category_id.is_none()
            && self.merchant_id.is_none()
            && self.exchange_rate_scaled.is_none()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ItemSnapshot {
    pub item_sequence: i32,
    pub name: String,
    pub unit_price: i64,
    pub quantity: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_type: Option<String>,
}

impl ItemSnapshot {
    pub fn from(item: &Item) -> Self {
        Self {
            item_sequence: item.item_sequence,
            name: item.name.clone(),
            unit_price: item.unit_price,
            quantity: item.quantity,
            item_type: item.item_type.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ItemDiff {
    pub item_sequence: i32,
    pub from: ItemSnapshot,
    pub to: ItemSnapshot,
}

impl ItemDiff {
    pub fn new(from: &Item, to: &Item) -> Self {
        Self {
            item_sequence: to.item_sequence,
            from: ItemSnapshot::from(from),
            to: ItemSnapshot::from(to),
        }
    }
}

/// Builds the field-change snapshot describing how `before` differs from the
/// fields requested in `updates`. Only fields explicitly provided (and actually
/// changed) are recorded.
pub fn diff_fields(before: &Purchase, updates: &PurchaseUpdates) -> PurchaseFieldChanges {
    let mut fields = PurchaseFieldChanges::default();

    if let Some(to) = &updates.store {
        if Some(to) != before.store.as_ref() {
            fields.store = Some(FieldChange {
                from: before.store.clone().unwrap_or_default(),
                to: to.clone(),
            });
        }
    }
    if let Some(to) = &updates.description {
        if Some(to) != before.description.as_ref() {
            fields.description = Some(FieldChange {
                from: before.description.clone().unwrap_or_default(),
                to: to.clone(),
            });
        }
    }
    if let Some(to) = &updates.notes {
        if Some(to) != before.notes.as_ref() {
            fields.notes = Some(FieldChange {
                from: before.notes.clone().unwrap_or_default(),
                to: to.clone(),
            });
        }
    }
    if let Some(to) = updates.category_id {
        if Some(to) != before.category_id {
            fields.category_id = Some(FieldChange {
                from: before.category_id,
                to: Some(to),
            });
        }
    }

    if let Some(to) = updates.merchant_id {
        if Some(to) != before.merchant_id {
            fields.merchant_id = Some(FieldChange {
                from: before.merchant_id,
                to: Some(to),
            });
        }
    }

    if let Some(to) = &updates.rate {
        if Some(to.rate_scaled) != before.exchange_rate_scaled {
            fields.exchange_rate_scaled = Some(FieldChange {
                from: before.exchange_rate_scaled,
                to: Some(to.rate_scaled),
            });
        }
    }

    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn purchase() -> Purchase {
        Purchase {
            id: uuid::Uuid::new_v4(),
            user_id: uuid::Uuid::new_v4(),
            wallet_id: uuid::Uuid::new_v4(),
            category_id: Some(1),
            classification_status: "confirmed".to_string(),
            merchant_id: None,
            store: Some("Store A".to_string()),
            description: Some("desc".to_string()),
            notes: Some("notes".to_string()),
            version: 5,
            currency: "USD".to_string(),
            exchange_rate_scaled: None,
            rate_source: None,
            rate_at: None,
            purchase_date: Utc::now(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        }
    }

    fn item(seq: i32, name: &str) -> Item {
        Item {
            item_sequence: seq,
            purchase_id: uuid::Uuid::new_v4(),
            name: name.to_string(),
            unit_price: 100,
            quantity: 2,
            item_type: Some("grocery".to_string()),
        }
    }

    fn updates(
        store: Option<&str>,
        description: Option<&str>,
        notes: Option<&str>,
        category_id: Option<i32>,
    ) -> PurchaseUpdates {
        PurchaseUpdates {
            store: store.map(str::to_string),
            description: description.map(str::to_string),
            notes: notes.map(str::to_string),
            category_id,
            merchant_id: None,
            rate: None,
        }
    }

    #[test]
    fn diff_fields_all_none_is_empty() {
        let fields = diff_fields(&purchase(), &updates(None, None, None, None));
        assert!(fields.is_empty());
    }

    #[test]
    fn diff_fields_records_only_changed_provided_fields() {
        let fields = diff_fields(
            &purchase(),
            &updates(Some("Store B"), None, Some("notes"), None),
        );
        assert!(!fields.is_empty());
        assert!(fields.description.is_none());
        assert!(fields.category_id.is_none());
        let store = fields.store.expect("store changed");
        assert_eq!(store.from, "Store A");
        assert_eq!(store.to, "Store B");
    }

    #[test]
    fn diff_fields_ignores_unchanged_provided_field() {
        // Same value as current => not recorded.
        let fields = diff_fields(&purchase(), &updates(Some("Store A"), None, None, None));
        assert!(fields.is_empty());
    }

    #[test]
    fn diff_fields_records_category_change() {
        let fields = diff_fields(&purchase(), &updates(None, None, None, Some(9)));
        assert!(fields.category_id.is_some());
        let cat = fields.category_id.unwrap();
        assert_eq!(cat.from, Some(1));
        assert_eq!(cat.to, Some(9));
        assert!(fields.store.is_none());
    }

    #[test]
    fn diff_fields_ignores_unchanged_category() {
        let fields = diff_fields(&purchase(), &updates(None, None, None, Some(1)));
        assert!(fields.is_empty());
    }

    #[test]
    fn diff_fields_clearing_to_empty_string_is_recorded() {
        // Provided Some("") differs from Some("notes") => recorded.
        let fields = diff_fields(&purchase(), &updates(None, None, Some(""), None));
        assert!(fields.notes.is_some());
        assert_eq!(fields.notes.unwrap().from, "notes");
    }

    #[test]
    fn item_snapshot_maps_all_fields() {
        let snap = ItemSnapshot::from(&item(3, "Milk"));
        assert_eq!(snap.item_sequence, 3);
        assert_eq!(snap.name, "Milk");
        assert_eq!(snap.unit_price, 100);
        assert_eq!(snap.quantity, 2);
        assert_eq!(snap.item_type, Some("grocery".to_string()));
    }

    #[test]
    fn item_diff_captures_before_and_after() {
        let mut before = item(3, "Milk");
        before.quantity = 1;
        let mut after = item(3, "Milk");
        after.quantity = 5;
        let diff = ItemDiff::new(&before, &after);
        assert_eq!(diff.item_sequence, 3);
        assert_eq!(diff.from.quantity, 1);
        assert_eq!(diff.to.quantity, 5);
    }

    #[test]
    fn purchase_changes_serializes_with_snake_case_action_tag() {
        let changes = PurchaseChanges::PurchaseUpdated {
            fields: diff_fields(&purchase(), &updates(Some("Store B"), None, None, None)),
        };
        let value = serde_json::to_value(&changes).unwrap();
        assert_eq!(value["action"], "purchase_updated");
        assert!(value["fields"]["store"].is_object());

        let deleted = PurchaseChanges::PurchaseDeleted;
        assert_eq!(
            serde_json::to_value(&deleted).unwrap()["action"],
            "purchase_deleted"
        );

        let added = PurchaseChanges::ItemAdded {
            items: vec![ItemSnapshot::from(&item(1, "Apple"))],
        };
        assert_eq!(
            serde_json::to_value(&added).unwrap()["action"],
            "item_added"
        );

        let removed = PurchaseChanges::ItemRemoved {
            items: vec![ItemSnapshot::from(&item(1, "Apple"))],
        };
        assert_eq!(
            serde_json::to_value(&removed).unwrap()["action"],
            "item_removed"
        );

        let updated = PurchaseChanges::ItemUpdated {
            items: vec![ItemDiff::new(&item(1, "Apple"), &item(1, "Banana"))],
        };
        assert_eq!(
            serde_json::to_value(&updated).unwrap()["action"],
            "item_updated"
        );
    }
}
