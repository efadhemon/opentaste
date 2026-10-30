package main

import (
	"fmt"
	"log/slog"
)

func run(batch []Job) {
	fmt.Println("synced", len(batch))
}

var _ = slog.Info
