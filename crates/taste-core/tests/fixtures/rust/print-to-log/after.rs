use tracing::info;

pub fn sync(batch: &[Job]) {
    info!("synced {}", batch.len());
}
