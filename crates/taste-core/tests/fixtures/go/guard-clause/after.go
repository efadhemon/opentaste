package orders

func Price(o *Order) int {
	if len(o.Items) == 0 {
		return o.Minimum
	}
	total := 0
	for _, item := range o.Items {
		total += item.Cost
	}
	return total
}
