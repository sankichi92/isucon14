use std::sync::Arc;
use std::time::Duration;

use async_stream::stream;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::Event;
use axum::response::Sse;
use axum_extra::extract::cookie::Cookie;
use axum_extra::extract::CookieJar;
use dashmap::DashMap;
use futures::Stream;
use sqlx::MySqlPool;
use tokio::sync::watch;
use tokio_stream::StreamExt as _;
use tracing::{info, warn};
use ulid::Ulid;

use crate::models::{Chair, ChairLocation, Owner, Ride, RideStatus, User};
use crate::{AppState, Coordinate, Error};

pub fn chair_routes(app_state: AppState) -> axum::Router<AppState> {
    let routes =
        axum::Router::new().route("/api/chair/chairs", axum::routing::post(chair_post_chairs));

    let authed_routes = axum::Router::new()
        .route(
            "/api/chair/activity",
            axum::routing::post(chair_post_activity),
        )
        .route(
            "/api/chair/coordinate",
            axum::routing::post(chair_post_coordinate),
        )
        .route(
            "/api/chair/notification",
            axum::routing::get(chair_get_notification),
        )
        .route(
            "/api/chair/rides/:ride_id/status",
            axum::routing::post(chair_post_ride_status),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            app_state.clone(),
            crate::middlewares::chair_auth_middleware,
        ));

    routes.merge(authed_routes)
}

#[derive(Debug, serde::Deserialize)]
struct ChairPostChairsRequest {
    name: String,
    model: String,
    chair_register_token: String,
}

#[derive(Debug, serde::Serialize)]
struct ChairPostChairsResponse {
    id: String,
    owner_id: String,
}

async fn chair_post_chairs(
    State(AppState { pool, .. }): State<AppState>,
    jar: CookieJar,
    axum::Json(req): axum::Json<ChairPostChairsRequest>,
) -> Result<(CookieJar, (StatusCode, axum::Json<ChairPostChairsResponse>)), Error> {
    let Some(owner): Option<Owner> =
        sqlx::query_as("SELECT * FROM owners WHERE chair_register_token = ?")
            .bind(req.chair_register_token)
            .fetch_optional(&pool)
            .await?
    else {
        return Err(Error::Unauthorized("invalid chair_register_token"));
    };

    let chair_id = Ulid::new().to_string();
    let access_token = crate::secure_random_str(32);

    sqlx::query("INSERT INTO chairs (id, owner_id, name, model, is_active, access_token) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(&chair_id)
        .bind(&owner.id)
        .bind(req.name)
        .bind(req.model)
        .bind(false)
        .bind(&access_token)
        .execute(&pool)
        .await?;

    let jar = jar.add(Cookie::build(("chair_session", access_token)).path("/"));

    Ok((
        jar,
        (
            StatusCode::CREATED,
            axum::Json(ChairPostChairsResponse {
                id: chair_id,
                owner_id: owner.id,
            }),
        ),
    ))
}

#[derive(Debug, serde::Deserialize)]
struct PostChairActivityRequest {
    is_active: bool,
}

async fn chair_post_activity(
    State(AppState { pool, .. }): State<AppState>,
    axum::Extension(chair): axum::Extension<Chair>,
    axum::Json(req): axum::Json<PostChairActivityRequest>,
) -> Result<StatusCode, Error> {
    sqlx::query("UPDATE chairs SET is_active = ? WHERE id = ?")
        .bind(req.is_active)
        .bind(chair.id)
        .execute(&pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, serde::Serialize)]
struct ChairPostCoordinateResponse {
    recorded_at: i64,
}

async fn chair_post_coordinate(
    State(AppState {
        pool,
        ride_status_notify_by_chair_id,
        ride_status_notify_by_user_id,
    }): State<AppState>,
    axum::Extension(chair): axum::Extension<Chair>,
    axum::Json(req): axum::Json<Coordinate>,
) -> Result<axum::Json<ChairPostCoordinateResponse>, Error> {
    let chair_location_id = Ulid::new().to_string();

    let mut tx = pool.begin().await?;

    let last_chair_location: Option<ChairLocation> = sqlx::query_as(
        "SELECT * FROM chair_locations WHERE chair_id = ? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&chair.id)
    .fetch_optional(&mut *tx)
    .await?;

    let total_distance = if let Some(last_location) = last_chair_location {
        last_location.total_distance
            + (req.latitude - last_location.latitude).abs()
            + (req.longitude - last_location.longitude).abs()
    } else {
        0
    };

    sqlx::query(
        r#"
        INSERT INTO
            chair_locations (id, chair_id, latitude, longitude, total_distance)
        VALUES
            (?, ?, ?, ?, ?)
        "#,
    )
    .bind(&chair_location_id)
    .bind(&chair.id)
    .bind(req.latitude)
    .bind(req.longitude)
    .bind(total_distance)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    let location: ChairLocation = sqlx::query_as("SELECT * FROM chair_locations WHERE id = ?")
        .bind(chair_location_id)
        .fetch_one(&pool)
        .await?;

    let mut tx = pool.begin().await?;

    let ride: Option<Ride> =
        sqlx::query_as("SELECT * FROM rides WHERE chair_id = ? ORDER BY updated_at DESC LIMIT 1")
            .bind(chair.id.clone())
            .fetch_optional(&mut *tx)
            .await?;
    if let Some(ride) = &ride {
        let status = crate::get_latest_ride_status(&mut *tx, &ride.id).await?;
        if status != "COMPLETED" && status != "CANCELED" {
            if req.latitude == ride.pickup_latitude
                && req.longitude == ride.pickup_longitude
                && status == "ENROUTE"
            {
                sqlx::query("INSERT INTO ride_statuses (id, ride_id, status) VALUES (?, ?, ?)")
                    .bind(Ulid::new().to_string())
                    .bind(&ride.id)
                    .bind("PICKUP")
                    .execute(&mut *tx)
                    .await?;
            }

            if req.latitude == ride.destination_latitude
                && req.longitude == ride.destination_longitude
                && status == "CARRYING"
            {
                sqlx::query("INSERT INTO ride_statuses (id, ride_id, status) VALUES (?, ?, ?)")
                    .bind(Ulid::new().to_string())
                    .bind(&ride.id)
                    .bind("ARRIVED")
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }

    tx.commit().await?;

    ride_status_notify_by_chair_id
        .entry(chair.id.clone())
        .or_insert_with(|| watch::channel(Ulid::new()))
        .0
        .send(Ulid::new())
        .unwrap();
    info!(chair_id = chair.id, "notify chair change");

    if let Some(ride) = &ride {
        ride_status_notify_by_user_id
            .entry(ride.user_id.clone())
            .or_insert_with(|| watch::channel(Ulid::new()))
            .0
            .send(Ulid::new())
            .unwrap();
        info!(user_id = ride.user_id, "notify user change");
    }

    Ok(axum::Json(ChairPostCoordinateResponse {
        recorded_at: location.created_at.timestamp_millis(),
    }))
}

#[derive(Debug, serde::Serialize)]
struct SimpleUser {
    id: String,
    name: String,
}

#[derive(Debug, serde::Serialize)]
struct ChairGetNotificationResponseData {
    ride_id: String,
    user: SimpleUser,
    pickup_coordinate: Coordinate,
    destination_coordinate: Coordinate,
    status: String,
}

fn chair_notification_stream(
    mut chair_notification: watch::Receiver<Ulid>,
    user_notify: Arc<DashMap<String, (watch::Sender<Ulid>, watch::Receiver<Ulid>)>>,
    pool: MySqlPool,
    chair_id: String,
) -> impl Stream<Item = Result<Option<ChairGetNotificationResponseData>, Error>> {
    info!(chair_id, "open new notification channel for chair");
    stream! {
        loop {
            let mut tx = pool.begin().await?;

            let Some(ride): Option<Ride> =
                sqlx::query_as("SELECT * FROM rides WHERE chair_id = ? ORDER BY updated_at DESC LIMIT 1")
                    .bind(&chair_id)
                    .fetch_optional(&mut *tx)
                    .await?
            else {
                tx.commit().await?;
                info!(chair_id, "no ride found");
                yield Ok(None);
                chair_notification.changed().await.unwrap();
                info!(chair_id, "got new ride");
                continue;
            };

            let yet_sent_ride_status: Option<RideStatus> =
                sqlx::query_as("SELECT * FROM ride_statuses WHERE ride_id = ? AND chair_sent_at IS NULL ORDER BY created_at ASC LIMIT 1")
                .bind(&ride.id)
                .fetch_optional(&mut *tx)
                .await?;
            let (yet_sent_ride_status_id, status) = if let Some(yet_sent_ride_status) = yet_sent_ride_status
            {
                (Some(yet_sent_ride_status.id), yet_sent_ride_status.status)
            } else {
                (None, crate::get_latest_ride_status(&mut *tx, &ride.id).await?,)
            };

            let user: User = sqlx::query_as("SELECT * FROM users WHERE id = ? FOR SHARE")
                .bind(ride.user_id.clone())
                .fetch_one(&mut *tx)
                .await?;

            if let Some(yet_sent_ride_status_id) = yet_sent_ride_status_id {
                sqlx::query("UPDATE ride_statuses SET chair_sent_at = CURRENT_TIMESTAMP(6) WHERE id = ?")
                    .bind(yet_sent_ride_status_id)
                    .execute(&mut *tx)
                    .await?;

                user_notify
                    .entry(ride.user_id.clone())
                    .or_insert_with(|| watch::channel(Ulid::new()))
                    .0
                    .send(Ulid::new())
                    .unwrap();
            }

            tx.commit().await?;
            info!(chair_id, status, "send sse");

            yield Ok(Some(ChairGetNotificationResponseData {
                ride_id: ride.id,
                user: SimpleUser {
                    id: user.id,
                    name: format!("{} {}", user.firstname, user.lastname),
                },
                pickup_coordinate: Coordinate {
                    latitude: ride.pickup_latitude,
                    longitude: ride.pickup_longitude,
                },
                destination_coordinate: Coordinate {
                    latitude: ride.destination_latitude,
                    longitude: ride.destination_longitude,
                },
                status,
            }));
            chair_notification.changed().await.unwrap();
        }
    }
}

async fn chair_get_notification(
    State(AppState {
        pool,
        ride_status_notify_by_chair_id,
        ride_status_notify_by_user_id,
        ..
    }): State<AppState>,
    axum::Extension(chair): axum::Extension<Chair>,
) -> Sse<impl Stream<Item = Result<Event, Error>>> {
    let chair_notification = ride_status_notify_by_chair_id
        .entry(chair.id.clone())
        .or_insert_with(|| watch::channel(Ulid::new()))
        .1
        .clone();

    let stream = chair_notification_stream(
        chair_notification,
        ride_status_notify_by_user_id,
        pool,
        chair.id.clone(),
    );
    let stream = stream.map(|result| match result {
        Ok(data) => Ok(Event::default().json_data(&data).unwrap()),
        Err(e) => {
            warn!(e = e.to_string(), "error happend");
            Err(e)
        }
    });

    Sse::new(stream.throttle(Duration::from_millis(300)))
}

#[derive(Debug, serde::Deserialize)]
struct PostChairRidesRideIDStatusRequest {
    status: String,
}

async fn chair_post_ride_status(
    State(AppState {
        pool,
        ride_status_notify_by_chair_id,
        ride_status_notify_by_user_id,
    }): State<AppState>,
    axum::Extension(chair): axum::Extension<Chair>,
    Path((ride_id,)): Path<(String,)>,
    axum::Json(req): axum::Json<PostChairRidesRideIDStatusRequest>,
) -> Result<StatusCode, Error> {
    let mut tx = pool.begin().await?;

    let Some(ride): Option<Ride> = sqlx::query_as("SELECT * FROM rides WHERE id = ? FOR UPDATE")
        .bind(ride_id)
        .fetch_optional(&mut *tx)
        .await?
    else {
        return Err(Error::NotFound("rides not found"));
    };

    if ride.chair_id.is_none_or(|chair_id| chair_id != chair.id) {
        return Err(Error::BadRequest("not assigned to this ride"));
    }

    match req.status.as_str() {
        // Acknowledge the ride
        "ENROUTE" => {
            sqlx::query("INSERT INTO ride_statuses (id, ride_id, status) VALUES (?, ?, ?)")
                .bind(Ulid::new().to_string())
                .bind(ride.id)
                .bind("ENROUTE")
                .execute(&mut *tx)
                .await?;
        }
        // After Picking up user
        "CARRYING" => {
            let status = crate::get_latest_ride_status(&mut *tx, &ride.id).await?;
            if status != "PICKUP" {
                return Err(Error::BadRequest("chair has not arrived yet"));
            }
            sqlx::query("INSERT INTO ride_statuses (id, ride_id, status) VALUES (?, ?, ?)")
                .bind(Ulid::new().to_string())
                .bind(ride.id)
                .bind("CARRYING")
                .execute(&mut *tx)
                .await?;
        }
        _ => {
            return Err(Error::BadRequest("invalid status"));
        }
    };

    tx.commit().await?;

    ride_status_notify_by_chair_id
        .entry(chair.id.clone())
        .or_insert_with(|| watch::channel(Ulid::new()))
        .0
        .send(Ulid::new())
        .unwrap();
    info!(chair_id = chair.id, "notify chair change");

    ride_status_notify_by_user_id
        .entry(ride.user_id.clone())
        .or_insert_with(|| watch::channel(Ulid::new()))
        .0
        .send(Ulid::new())
        .unwrap();
    info!(user_id = ride.user_id, "notify chair change");

    Ok(StatusCode::NO_CONTENT)
}
