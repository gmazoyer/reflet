use std::convert::Infallible;
use std::sync::atomic::Ordering;

use async_stream::stream;
use axum::Json;
use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use reflet_core::event_log::{RouteEvent, RouteEventType};

use crate::state::AppState;

#[derive(Debug, Deserialize, IntoParams)]
pub struct EventsQuery {
    /// Only return events with seq > this value (cursor-based polling).
    pub since_seq: Option<u64>,
    /// Filter by peer ID.
    pub peer_id: Option<String>,
    /// Filter by event type (announce, withdraw, session_up, session_down).
    #[serde(rename = "type")]
    pub event_type: Option<String>,
    /// Maximum number of events to return (default 1000, max 10000).
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EventsResponse {
    pub events: Vec<RouteEvent>,
    pub current_seq: u64,
    pub count: usize,
}

/// List recent route change events.
#[utoipa::path(
    get,
    path = "/api/v1/events",
    params(EventsQuery),
    responses(
        (status = 200, description = "List of route events", body = EventsResponse),
        (status = 400, description = "Invalid filter parameter"),
    ),
    tag = "events"
)]
pub async fn get_events(
    State(state): State<AppState>,
    Query(params): Query<EventsQuery>,
) -> Result<Json<EventsResponse>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    let event_type = match &params.event_type {
        Some(t) => {
            let parsed = RouteEventType::parse(t);
            if parsed.is_none() {
                return Err((
                    axum::http::StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!("invalid event type: {t}. Valid types: announce, withdraw, session_up, session_down")
                    })),
                ));
            }
            parsed
        }
        None => None,
    };

    let limit = params.limit.unwrap_or(1000).min(10_000);

    let events = state.event_log.query(
        params.since_seq,
        params.peer_id.as_deref(),
        event_type,
        limit,
    );
    let current_seq = state.event_log.current_seq();
    let count = events.len();

    Ok(Json(EventsResponse {
        events,
        current_seq,
        count,
    }))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct EventStreamQuery {
    /// Resume from this sequence number.
    pub since_seq: Option<u64>,
    /// Filter by peer ID.
    pub peer_id: Option<String>,
    /// Filter by event type (announce, withdraw, session_up, session_down).
    #[serde(rename = "type")]
    pub event_type: Option<String>,
}

/// Stream route change events via Server-Sent Events.
#[utoipa::path(
    get,
    path = "/api/v1/events/stream",
    params(EventStreamQuery),
    responses(
        (status = 200, description = "SSE event stream", content_type = "text/event-stream"),
    ),
    tag = "events"
)]
pub async fn event_stream(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Query(params): Query<EventStreamQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Determine initial cursor: query param takes precedence, then Last-Event-ID header
    let mut last_seq = params.since_seq.or_else(|| {
        headers
            .get("Last-Event-ID")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
    });

    let event_type = params.event_type.as_deref().and_then(RouteEventType::parse);
    let peer_id = params.peer_id;

    let stream = stream! {
        loop {
            let events = state.event_log.query(
                last_seq,
                peer_id.as_deref(),
                event_type,
                1000,
            );

            for event in &events {
                let event_name = match event.event_type {
                    RouteEventType::Announce => "announce",
                    RouteEventType::Withdraw => "withdraw",
                    RouteEventType::SessionUp => "session_up",
                    RouteEventType::SessionDown => "session_down",
                };

                if let Ok(data) = serde_json::to_string(event) {
                    yield Ok(Event::default()
                        .event(event_name)
                        .data(data)
                        .id(event.seq.to_string()));
                }

                last_seq = Some(event.seq);
            }

            // Wait for new events or shutdown
            state.event_notify.notified().await;
            if state.shutdown.load(Ordering::Relaxed) {
                break;
            }
        }
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
}
