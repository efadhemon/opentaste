pub fn price(order: &Order) -> u64 {
    if let Some(items) = order.items() {
        items.iter().map(Item::cost).sum()
    } else {
        0
    }
}
