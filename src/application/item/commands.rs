#[derive(Debug)]
pub struct CreateItemCommand {
    pub name: String,
    pub unit_price: f64,
    pub quantity: i32,
    pub item_type: Option<String>,
}

#[derive(Debug)]
pub struct UpdateItemCommand {
    pub name: Option<String>,
    pub unit_price: Option<String>,
    pub quantity: Option<i32>,
    pub item_type: Option<String>,
}
