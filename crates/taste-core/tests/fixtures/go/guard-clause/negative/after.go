package orders

func Price(o *Order) (int, error) {
	if err := o.Validate(); err != nil {
		return 0, err
	}
	total := 0
	for _, item := range o.Items {
		total += item.Cost
	}
	return total, nil
}
