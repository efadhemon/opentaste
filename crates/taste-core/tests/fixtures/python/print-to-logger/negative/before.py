import logging

logger = logging.getLogger(__name__)


def sync(batch):
    print("synced", len(batch))
