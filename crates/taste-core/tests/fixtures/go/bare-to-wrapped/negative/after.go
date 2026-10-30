package config

import "fmt"

func (s *Store) Load(path string) ([]byte, error) {
	data, err := s.read(path)
	if err != nil {
		return nil, fmt.Errorf("load config %s: %w", path, err)
	}
	return data, nil
}

var _ = fmt.Sprint
