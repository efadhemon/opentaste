def price(order):
    if not order.items:
        return 0
    return sum(item.cost for item in order.items)
