use crate::infrai::{InfraiClient, InfraiError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct StartSession {
    pub session_id: String,
    pub player_id: String,
    pub display_name: String,
    pub content_type: String,
    pub max_bytes: u64,
    #[serde(default)]
    pub needs_moderation: bool,
}

#[derive(Debug, Serialize)]
pub struct SessionHandoff {
    pub room: String,
    pub room_token: String,
    pub upload_url: String,
    pub object_key: String,
    pub queue: ArchiveQueue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveQueue {
    SessionArchive,
    ModerationPending,
}

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("session_id, player_id, and display_name must be non-empty")]
    InvalidIdentity,
    #[error("max_bytes must be greater than zero")]
    InvalidSize,
    #[error(transparent)]
    Infrai(#[from] InfraiError),
}

use thiserror::Error;

#[derive(Clone)]
pub struct SessionArchive {
    infrai: InfraiClient,
    bucket: String,
}

impl SessionArchive {
    pub fn new(infrai: InfraiClient, bucket: String) -> Self {
        Self { infrai, bucket }
    }

    pub async fn prepare(&self) -> Result<(), SessionError> {
        self.infrai.ensure_bucket(&self.bucket).await?;
        Ok(())
    }

    pub async fn start(&self, input: StartSession) -> Result<SessionHandoff, SessionError> {
        validate(&input)?;
        let plan = archive_plan(&input);
        let event_id = format!("asset-ready:{}:{}", input.session_id, input.player_id);

        self.infrai.create_room(&input.session_id).await?;
        let token = self
            .infrai
            .issue_room_token(&input.session_id, &input.player_id, &input.display_name)
            .await?;
        let upload = self
            .infrai
            .presign_upload(
                &self.bucket,
                &plan.object_key,
                &input.content_type,
                input.max_bytes,
                &event_id,
            )
            .await?;
        self.infrai
            .publish_asset_ready(
                &format!("game-session:{}", input.session_id),
                &event_id,
                &plan.object_key,
                &input.player_id,
            )
            .await?;

        Ok(SessionHandoff {
            room: input.session_id,
            room_token: token.token,
            upload_url: upload.url,
            object_key: plan.object_key,
            queue: plan.queue,
        })
    }
}

struct ArchivePlan {
    object_key: String,
    queue: ArchiveQueue,
}

fn archive_plan(input: &StartSession) -> ArchivePlan {
    let (prefix, queue) = if input.needs_moderation {
        ("moderation/pending", ArchiveQueue::ModerationPending)
    } else {
        ("sessions", ArchiveQueue::SessionArchive)
    };
    ArchivePlan {
        object_key: format!("{}/{}/{}.webm", prefix, input.session_id, input.player_id),
        queue,
    }
}

fn validate(input: &StartSession) -> Result<(), SessionError> {
    if input.session_id.trim().is_empty()
        || input.player_id.trim().is_empty()
        || input.display_name.trim().is_empty()
    {
        return Err(SessionError::InvalidIdentity);
    }
    if input.max_bytes == 0 {
        return Err(SessionError::InvalidSize);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(needs_moderation: bool) -> StartSession {
        StartSession {
            session_id: "match-2048".to_owned(),
            player_id: "player-7".to_owned(),
            display_name: "Mira".to_owned(),
            content_type: "video/webm".to_owned(),
            max_bytes: 50_000_000,
            needs_moderation,
        }
    }

    #[test]
    fn routes_flagged_assets_to_the_moderation_queue() {
        let flagged = archive_plan(&input(true));
        let ordinary = archive_plan(&input(false));

        assert_eq!(flagged.queue, ArchiveQueue::ModerationPending);
        assert_eq!(
            flagged.object_key,
            "moderation/pending/match-2048/player-7.webm"
        );
        assert_eq!(ordinary.queue, ArchiveQueue::SessionArchive);
        assert_eq!(ordinary.object_key, "sessions/match-2048/player-7.webm");
    }
}
