import logging

logger = logging.getLogger(__name__)


def sync(batch):
    logger.info("synced", len(batch))
