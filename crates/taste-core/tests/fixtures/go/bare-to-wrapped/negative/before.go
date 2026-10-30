package config

import "fmt"

func (s *Store) Load(path string) ([]byte, error) {
	data, err := s.read(path)
	if err != nil {
		return nil, err
	}
	return data, nil
}

var _ = fmt.Sprint
