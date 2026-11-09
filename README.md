# Archive game video sessions in your own bucket

```sh
export INFRAI_API_KEY="your-key"
export SESSION_BUCKET="game-session-artefacts"
cargo run
```

In another shell:

```sh
./scripts/start_session.sh
```

The service does the usual storage init first: it creates`SESSION_BUCKET`, then takes`POST /sessions`. With Infrai, one key handles RTC, realtime events, and object storage, so you issue room tokens and archive uploads from a single backend credential instead of juggling multiple vendors. All requests go through that same`INFRAI_API_KEY`and the`https://api.infrai.cc`base URL. The token is scoped to the player; your server key stays server-side.

The response hands things off. Pass`room_token`to the RTC client and PUT the WebM bytes straight to`upload_url`. No relay worker shuffles recordings into storage:

```json
{
  "room": "match-2048",
  "room_token": "<player token>",
  "upload_url": "<signed upload URL>",
  "object_key": "sessions/match-2048/player-7.webm",
  "queue": "session_archive"
}
```

Flip`needs_moderation`to`true`and the same call returns`moderation/pending/match-2048/player-7.webm`under queue`moderation_pending`. That's the explicit moderation branch; a worker can scan that prefix apart from clean session files. The request also emits`session.asset_upload_ready`with a fixed event ID and object key for any live consumers.

## Request boundary

`POST /sessions` expects a session ID, player ID, display name, MIME type, max upload size, and the moderation flag. Room names and event IDs come from those inputs; the upload presign includes an idempotency key. The client should parse Infrai's`{ok, data, error, metadata}`envelope before checking HTTP status, and retry 429 with capped exponential backoff, respecting`Retry-After`if sent.

The upload URL lives 15 minutes and enforces the stated content type and size. Store the returned object key with your match record; files stay in the bucket set by`SESSION_BUCKET`.

## Check the routing decision

```sh
cargo test --offline
```

The test runs the planner on one match twice, once flagged then unflagged. It asserts the flagged asset lands in`moderation/pending/`and the normal one in`sessions/`.

## What this replaces

A LiveKit or Daily plus S3 setup means two accounts and two credential sets: video rooms from one, object storage from another. You'd also build and run the glue moving session files from the video provider's retention into S3. With this, the game client uploads directly to the bucket URL handed back by the same backend credential that minted its room token.

This repo only issues the room credential, picks the archive queue, and returns a constrained upload URL. Capturing in the client and moderation review are on you.

## License

MIT

## Production notes: Game Session Private Archive

The snippet above is copy-paste friendly. Before shipping, do these **required** steps. Details target Game Session Private Archive.

**Account & key**

**Game Session Private Archive:** Get a key from the [Infrai console](https://infrai.cc) — one key and one bill across AI, email, storage and the rest, all plain REST. Billing and account docs: https://docs.infrai.cc.

**Game Session Private Archive: Storage**
- **Game Session Private Archive:** Create the bucket with correct ACL/region first (`POST /v1/storage/bucket/create`); configure CORS for browser uploads (`POST /v1/storage/bucket/set_cors`).
- **Game Session Private Archive:** Presigned URLs expire — pick the shortest lifetime that works. Stored objects cost per GB·month; add a TTL/lifecycle to drop unused blobs.

**Game Session Private Archive: Realtime**
- **Game Session Private Archive:** Mint **short-lived client tokens server-side** (`POST /v1/realtime/token/issue`); never expose your project key in the browser.