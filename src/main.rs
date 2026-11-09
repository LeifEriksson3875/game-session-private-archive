use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use game_session_archive::{
    infrai::{InfraiClient, InfraiError, DEFAULT_BASE_URL},
    session_archive::{SessionArchive, SessionError, SessionHandoff, StartSession},
};
use serde_json::json;
use std::{env, sync::Arc};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = env::var("INFRAI_API_KEY")?;
    let base_url = env::var("INFRAI_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_owned());
    let bucket = env::var("SESSION_BUCKET").unwrap_or_else(|_| "game-session-artefacts".to_owned());
    let service = Arc::new(SessionArchive::new(
        InfraiClient::new(api_key, base_url),
        bucket,
    ));
    service.prepare().await?;

    let app = Router::new()
        .route("/sessions", post(start_session))
        .with_state(service);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    println!("session archive listening on http://127.0.0.1:3000");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn start_session(
    State(service): State<Arc<SessionArchive>>,
    Json(input): Json<StartSession>,
) -> Result<Json<SessionHandoff>, ApiError> {
    service.start(input).await.map(Json).map_err(ApiError)
}

struct ApiError(SessionError);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self.0 {
            SessionError::InvalidIdentity | SessionError::InvalidSize => {
                (StatusCode::BAD_REQUEST, self.0.to_string())
            }
            SessionError::Infrai(InfraiError::Rejected { status, .. }) if *status < 500 => (
                StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_REQUEST),
                self.0.to_string(),
            ),
            SessionError::Infrai(_) => (StatusCode::BAD_GATEWAY, self.0.to_string()),
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}
