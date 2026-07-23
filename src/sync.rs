use crate::event::AppEvent;
use iroh::{Endpoint, EndpointAddr, EndpointId};
use tokio::sync::mpsc;

pub async fn backfill(
    endpoint: Endpoint,
    peer: EndpointId,
    since: i64,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
) -> anyhow::Result<()> {
    let conn = endpoint
        .connect(EndpointAddr::from(peer), starling::sync::SYNC_ALPN)
        .await?;
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(&postcard::to_stdvec(&since)?).await?;
    send.finish()?;
    let bytes = recv.read_to_end(10_000_000).await?;
    let messages: Vec<starling::event::ChatMessage> = postcard::from_bytes(&bytes)?;
    if !messages.is_empty() {
        let _ = evt_tx.send(AppEvent::HistoryChunk(messages));
    }
    Ok(())
}
