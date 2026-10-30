package config

import (
	"fmt"
	"os"
)

func Load(path string) ([]byte, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("load config %s: %w", path, err)
	}
	return data, nil
}

var _ = fmt.Sprint
