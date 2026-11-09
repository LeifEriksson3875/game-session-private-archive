#!/bin/sh
set -eu

curl --fail-with-body \
  --request POST \
  --header 'content-type: application/json' \
  --data '{"session_id":"match-2048","player_id":"player-7","display_name":"Mira","content_type":"video/webm","max_bytes":50000000,"needs_moderation":false}' \
  http://127.0.0.1:3000/sessions

