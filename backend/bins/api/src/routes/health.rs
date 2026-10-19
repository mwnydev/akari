use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct Health {
    status: &'static str,
}

/// 存活检查。
#[utoipa::path(get, path = "/api/v1/healthz", tag = "system", responses((status = 200, body = Health)))]
pub async fn healthz() -> Json<Health> {
    Json(Health { status: "ok" })
}
