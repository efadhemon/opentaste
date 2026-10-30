package main

import (
	"fmt"
	"log/slog"
)

func run(batch []Job) {
	slog.Info("synced", "jobs", len(batch))
}

var _ = fmt.Sprint
