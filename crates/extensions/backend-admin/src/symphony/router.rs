use axum::extract::State;
use axum::response::sse::{Event, Sse};
use axum::routing::get;
use axum::{Json, Router};
use futures::StreamExt;
use rara_symphony::status::{SymphonySnapshot, SymphonyStatusHandle};

pub fn symphony_routes(handle: SymphonyStatusHandle) -> Router {
    Router::new()
        .route("/api/symphony/status", get(get_status))
        .route("/api/symphony/events", get(event_stream))
        .with_state(handle)
}

async fn get_status(State(handle): State<SymphonyStatusHandle>) -> Json<SymphonySnapshot> {
    let snapshot = handle.state.read().await;
    Json(snapshot.clone())
}

async fn event_stream(
    State(handle): State<SymphonyStatusHandle>,
) -> Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let rx = handle.subscribe_events();
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(|result| async {
        match result {
            Ok(log) => {
                let kind = log.kind.clone();
                let data = serde_json::to_string(&log).unwrap_or_default();
                Some(Ok(Event::default().event(kind).data(data)))
            }
            Err(_) => None, // lagged, skip
        }
    });
    Sse::new(stream)
}
