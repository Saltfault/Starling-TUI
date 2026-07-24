use crate::event::AppEvent;
use iroh::{Endpoint, EndpointAddr, EndpointId};
use serde::Serialize;
use tokio::sync::mpsc;

#[derive(Serialize)]
struct RoostSyncRequest<'a> {
    channel: &'a str,
    since: i64,
}

pub async fn backfill(
    endpoint: Endpoint,
    peer: EndpointId,
    flock: String,
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
        let _ = evt_tx.send(AppEvent::HistoryChunk { flock, messages });
    }
    Ok(())
}

pub async fn backfill_roost_channel(
    endpoint: Endpoint,
    peer: EndpointId,
    roost_code: &str,
    channel: &str,
    since: i64,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
) -> anyhow::Result<()> {
    const ROOST_SYNC_ALPN: &[u8] = b"starling/roost-sync/0";

    let conn = endpoint
        .connect(EndpointAddr::from(peer), ROOST_SYNC_ALPN)
        .await?;
    let (mut send, mut recv) = conn.open_bi().await?;
    let request = RoostSyncRequest { channel, since };
    send.write_all(&postcard::to_stdvec(&request)?).await?;
    send.finish()?;
    let bytes = recv.read_to_end(10_000_000).await?;
    let messages: Vec<starling::event::ChatMessage> = postcard::from_bytes(&bytes)?;
    if !messages.is_empty() {
        let flock = format!("{roost_code}/{channel}");
        let _ = evt_tx.send(AppEvent::HistoryChunk { flock, messages });
    }
    Ok(())
}
