def price(order):
    if order.items:
        total = sum(item.cost for item in order.items)
    else:
        total = 0
    return total
