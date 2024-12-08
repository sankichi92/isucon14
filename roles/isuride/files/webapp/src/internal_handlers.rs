use axum::extract::State;
use axum::http::StatusCode;

use crate::models::{Chair, Ride};
use crate::{AppState, Error};

pub fn internal_routes() -> axum::Router<AppState> {
    axum::Router::new().route(
        "/api/internal/matching",
        axum::routing::get(internal_get_matching),
    )
}

// このAPIをインスタンス内から一定間隔で叩かせることで、椅子とライドをマッチングさせる
pub async fn internal_get_matching(
    State(AppState { pool, .. }): State<AppState>,
) -> Result<StatusCode, Error> {
    // MEMO: 一旦最も待たせているリクエストに適当な空いている椅子マッチさせる実装とする。おそらくもっといい方法があるはず…
    let Some(ride): Option<Ride> =
        sqlx::query_as("SELECT * FROM rides WHERE chair_id IS NULL ORDER BY created_at LIMIT 1")
            .fetch_optional(&*pool)
            .await?
    else {
        return Ok(StatusCode::NO_CONTENT);
    };

    let matched: Vec<Chair> =
        sqlx::query_as("SELECT chairs.*, chair_models.speed FROM chairs INNER JOIN chair_models ON chairs.model = chair_models.name WHERE chairs.is_active = TRUE ORDER BY chair_models.speed DESC LIMIT 10")
            .fetch_all(&*pool)
            .await?;

    for m in matched {
        let empty: bool = sqlx::query_scalar(
            "SELECT COUNT(*) = 0 FROM (SELECT COUNT(chair_sent_at) = 6 AS completed FROM ride_statuses WHERE ride_id IN (SELECT id FROM rides WHERE chair_id = ?) GROUP BY ride_id) is_completed WHERE completed = FALSE",
        )
        .bind(&m.id)
        .fetch_one(&*pool)
        .await?;

        if empty {
            sqlx::query("UPDATE rides SET chair_id = ? WHERE id = ?")
                .bind(m.id)
                .bind(ride.id)
                .execute(&*pool)
                .await?;
            break;
        }
    }

    Ok(StatusCode::NO_CONTENT)
}
