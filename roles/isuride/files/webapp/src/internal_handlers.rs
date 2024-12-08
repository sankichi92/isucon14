use std::i32;

use axum::extract::State;
use axum::http::StatusCode;
use tokio::sync::watch;
use tracing::info;
use ulid::Ulid;

use crate::models::{Chair, ChairAndLocation, Ride};
use crate::{AppState, Error};

pub fn internal_routes() -> axum::Router<AppState> {
    axum::Router::new().route(
        "/api/internal/matching",
        axum::routing::get(internal_get_matching),
    )
}

// このAPIをインスタンス内から一定間隔で叩かせることで、椅子とライドをマッチングさせる
pub async fn internal_get_matching(
    State(AppState {
        pool,
        ride_status_notify_by_chair_id,
        ride_status_notify_by_user_id,
    }): State<AppState>,
) -> Result<StatusCode, Error> {

    let rides: Vec<Ride> =
        sqlx::query_as("SELECT * FROM rides WHERE chair_id IS NULL ORDER BY created_at LIMIT 10")
            .fetch_all(&pool)
            .await?;
    if rides.len() == 0 {
        return Ok(StatusCode::NO_CONTENT);
    };

    // active で速い椅子を選んでくる
    let candidate: Vec<ChairAndLocation> =
        sqlx::query_as("SELECT chairs.*, chair_models.speed, chair_locations.* FROM chairs INNER JOIN chair_locations ON chairs.id = chair_locations.chair_id INNER JOIN chair_models ON chairs.model = chair_models.name WHERE chairs.is_active = TRUE ORDER BY chair_models.speed DESC LIMIT 10")
            .fetch_all(&pool)
            .await?;

    for r in rides {
        let mut closest = None;
        let mut min_distance = i32::MAX;
        for c in candidate.iter() {
            let d = crate::calculate_distance(r.pickup_latitude, r.pickup_longitude, c.latitude, c.longitude);
            if d < min_distance {
                min_distance = d;
                closest = Some(c);
            }
        }

        let closest = closest.unwrap();
        sqlx::query("UPDATE rides SET chair_id = ? WHERE id = ?")
                .bind(closest.id.clone())
                .bind(r.id)
                .execute(&pool)
                .await?;

        ride_status_notify_by_chair_id
            .entry(closest.id.clone())
            .or_insert_with(|| watch::channel(Ulid::new()))
            .0
            .send(Ulid::new())
            .unwrap();
        info!(chair_id = closest.id, "notify chair change");
        ride_status_notify_by_user_id
            .entry(r.user_id.clone())
            .or_insert_with(|| watch::channel(Ulid::new()))
            .0
            .send(Ulid::new())
            .unwrap();
    }

    Ok(StatusCode::NO_CONTENT)
}
