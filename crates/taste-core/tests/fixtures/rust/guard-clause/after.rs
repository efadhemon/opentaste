pub fn price(order: &Order) -> u64 {
    let Some(items) = order.items() else {
        return 0;
    };
    items.iter().map(Item::cost).sum()
}
