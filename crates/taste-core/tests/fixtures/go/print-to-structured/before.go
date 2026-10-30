package sync

import (
	"fmt"
	"log/slog"
)

func Run(batch []Job) {
	fmt.Println("synced", len(batch))
}

var _ = slog.Info
