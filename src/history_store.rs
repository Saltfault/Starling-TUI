use anyhow::{Context, ensure};
use iroh::EndpointId;
use serde::{Deserialize, Serialize};
use sled::transaction::{ConflictableTransactionError, Transactional};
use starling::history::{HistoryHead, TrustedEvent, TrustedStore};
use starling::membership::MembershipState;
use starling::protocol::{EventHash, SignedEventV1, SpaceId};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};

const SCHEMA_VERSION: &[u8] = b"1";
const SCHEMA_KEY: &[u8] = b"history-store";

#[derive(Clone)]
pub struct SledHistory {
    db: sled::Db,
    events: sled::Tree,
    space_index: sled::Tree,
    session_heads: sled::Tree,
    heads: sled::Tree,
    schema: sled::Tree,
    memberships: Arc<RwLock<HashMap<SpaceId, MembershipState>>>,
}

#[derive(Serialize, Deserialize)]
struct StoredHead {
    frontier: Vec<EventHash>,
    event_count: u64,
}

impl SledHistory {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        Self::open_with_memberships(path, Arc::new(RwLock::new(HashMap::new())))
    }

    pub fn open_with_memberships(
        path: impl AsRef<Path>,
        memberships: Arc<RwLock<HashMap<SpaceId, MembershipState>>>,
    ) -> anyhow::Result<Self> {
        let db = sled::open(path).context("failed to open history database")?;
        let store = Self {
            events: db.open_tree("events")?,
            space_index: db.open_tree("space_index")?,
            session_heads: db.open_tree("session_heads")?,
            heads: db.open_tree("heads")?,
            schema: db.open_tree("schema")?,
            db,
            memberships,
        };
        match store.schema.get(SCHEMA_KEY)? {
            Some(version) => ensure!(
                version.as_ref() == SCHEMA_VERSION,
                "unsupported history schema"
            ),
            None => {
                store.schema.insert(SCHEMA_KEY, SCHEMA_VERSION)?;
                store.db.flush().context("failed to flush history schema")?;
            }
        }
        Ok(store)
    }

    #[allow(dead_code)]
    pub fn set_membership(
        &self,
        space: SpaceId,
        membership: MembershipState,
    ) -> anyhow::Result<()> {
        ensure!(
            membership_matches(space, &membership),
            "membership scope does not match space"
        );
        self.memberships
            .write()
            .map_err(|_| anyhow::anyhow!("membership cache lock poisoned"))?
            .insert(space, membership);
        Ok(())
    }
}

fn encode<T: Serialize>(value: &T) -> anyhow::Result<Vec<u8>> {
    postcard::to_stdvec(value).context("failed to encode history key")
}

fn event_key(space: &SpaceId, hash: &EventHash) -> anyhow::Result<Vec<u8>> {
    encode(&(*space, *hash))
}

fn sequence_key(
    space: &SpaceId,
    sender: &EndpointId,
    session: &[u8; 16],
    sequence: u64,
) -> anyhow::Result<Vec<u8>> {
    encode(&(*space, *sender, *session, sequence))
}

fn sender_key(space: &SpaceId, sender: &EndpointId) -> anyhow::Result<Vec<u8>> {
    encode(&(*space, *sender))
}

fn decode_head(bytes: &[u8]) -> anyhow::Result<HistoryHead> {
    let stored: StoredHead = postcard::from_bytes(bytes).context("invalid stored history head")?;
    HistoryHead::new(stored.frontier, stored.event_count)
}

fn encode_head(head: &HistoryHead) -> anyhow::Result<Vec<u8>> {
    encode(&StoredHead {
        frontier: head.frontier.clone(),
        event_count: head.event_count,
    })
}

fn membership_matches(space: SpaceId, membership: &MembershipState) -> bool {
    use starling::membership::MembershipScopeId;
    match (space, membership.scope()) {
        (SpaceId::Flock(a), MembershipScopeId::Flock(b)) => a == b,
        (SpaceId::RoostChannel { roost: a, .. }, MembershipScopeId::Roost(b)) => a == b,
        _ => false,
    }
}

impl TrustedStore for SledHistory {
    fn head(&self, space: &SpaceId) -> anyhow::Result<HistoryHead> {
        let key = encode(space)?;
        self.heads
            .get(key)?
            .map(|bytes| decode_head(&bytes))
            .transpose()
            .map(|head| head.unwrap_or_else(HistoryHead::empty))
    }

    fn event(&self, space: &SpaceId, hash: &EventHash) -> anyhow::Result<Option<TrustedEvent>> {
        let Some(encoded) = self.events.get(event_key(space, hash)?)? else {
            return Ok(None);
        };
        let event: SignedEventV1 =
            postcard::from_bytes(&encoded).context("invalid stored signed event")?;
        ensure!(
            postcard::to_stdvec(&event)?.as_slice() == encoded.as_ref(),
            "stored event is not canonical"
        );
        ensure!(event.event.space == *space, "stored event space mismatch");
        ensure!(event.verify()? == *hash, "stored event hash mismatch");
        Ok(Some(TrustedEvent {
            hash: *hash,
            encoded: encoded.to_vec(),
            event,
        }))
    }

    fn sequence_hash(
        &self,
        space: &SpaceId,
        sender: &EndpointId,
        session: &[u8; 16],
        sequence: u64,
    ) -> anyhow::Result<Option<EventHash>> {
        self.space_index
            .get(sequence_key(space, sender, session, sequence)?)?
            .map(|bytes| {
                bytes
                    .as_ref()
                    .try_into()
                    .context("invalid sequence hash length")
            })
            .transpose()
    }

    fn sender_head(
        &self,
        space: &SpaceId,
        sender: &EndpointId,
    ) -> anyhow::Result<Option<EventHash>> {
        self.session_heads
            .get(sender_key(space, sender)?)?
            .map(|bytes| {
                bytes
                    .as_ref()
                    .try_into()
                    .context("invalid sender head length")
            })
            .transpose()
    }

    fn membership(&self, space: &SpaceId) -> anyhow::Result<MembershipState> {
        self.memberships
            .read()
            .map_err(|_| anyhow::anyhow!("membership cache lock poisoned"))?
            .get(space)
            .cloned()
            .context("membership state is unavailable")
    }

    fn commit(
        &self,
        space: &SpaceId,
        expected: &HistoryHead,
        events: &[TrustedEvent],
        new_head: &HistoryHead,
    ) -> anyhow::Result<()> {
        let head_key = encode(space)?;
        let expected_bytes = encode_head(expected)?;
        let new_head_bytes = encode_head(new_head)?;
        let empty = HistoryHead::empty();
        let staged = events
            .iter()
            .map(|trusted| {
                ensure!(
                    trusted.event.event.space == *space,
                    "commit event space mismatch"
                );
                ensure!(
                    trusted.event.verify()? == trusted.hash,
                    "commit event hash mismatch"
                );
                ensure!(
                    postcard::to_stdvec(&trusted.event)? == trusted.encoded,
                    "commit event is not canonical"
                );
                Ok((
                    event_key(space, &trusted.hash)?,
                    sequence_key(
                        space,
                        &trusted.event.event.sender,
                        &trusted.event.event.session_id,
                        trusted.event.event.sequence,
                    )?,
                    sender_key(space, &trusted.event.event.sender)?,
                    trusted.hash,
                    trusted.encoded.clone(),
                ))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        (
            &self.events,
            &self.space_index,
            &self.session_heads,
            &self.heads,
        )
            .transaction(|(event_tree, sequence_tree, sender_tree, head_tree)| {
                let actual = head_tree.get(head_key.as_slice())?;
                let head_matches = match actual.as_ref() {
                    Some(bytes) => bytes.as_ref() == expected_bytes.as_slice(),
                    None => expected == &empty,
                };
                if !head_matches {
                    return Err(ConflictableTransactionError::Abort(
                        "history head changed during validation".to_owned(),
                    ));
                }
                for (event_key, sequence_key, sender_key, hash, encoded) in &staged {
                    if event_tree.get(event_key.as_slice())?.is_some() {
                        return Err(ConflictableTransactionError::Abort(
                            "event became present during commit".to_owned(),
                        ));
                    }
                    if sequence_tree.get(sequence_key.as_slice())?.is_some() {
                        return Err(ConflictableTransactionError::Abort(
                            "sequence became present during commit".to_owned(),
                        ));
                    }
                    event_tree.insert(event_key.as_slice(), encoded.as_slice())?;
                    sequence_tree.insert(sequence_key.as_slice(), hash.as_slice())?;
                    sender_tree.insert(sender_key.as_slice(), hash.as_slice())?;
                }
                head_tree.insert(head_key.as_slice(), new_head_bytes.as_slice())?;
                Ok(())
            })
            .map_err(|error| anyhow::anyhow!("atomic history transaction failed: {error}"))?;
        self.db.flush().context("failed to flush history commit")?;
        Ok(())
    }
}
