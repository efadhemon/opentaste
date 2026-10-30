package orders

func Price(o *Order) int {
	total := 0
	if len(o.Items) > 0 {
		for _, item := range o.Items {
			total += item.Cost
		}
	} else {
		total = o.Minimum
	}
	return total
}
