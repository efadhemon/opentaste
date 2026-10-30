package sync

import (
	"fmt"
	"log/slog"
)

func Run(batch []Job) {
	slog.Info("synced", "jobs", len(batch))
}

var _ = fmt.Sprint
