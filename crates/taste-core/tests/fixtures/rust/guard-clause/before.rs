pub fn price(order: &Order) -> u64 {
    let total;
    if let Some(items) = order.items() {
        total = items.iter().map(Item::cost).sum();
    } else {
        total = 0;
    }
    total
}
