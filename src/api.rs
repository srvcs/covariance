use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::{OpenApi, ToSchema};

use crate::client::{self, DepError};

pub const SERVICE: &str = "srvcs-covariance";
pub const CONCERN: &str = "statistics: population covariance";
pub const DEPENDS_ON: &[&str] = &[
    "srvcs-sum",
    "srvcs-floatdivide",
    "srvcs-floatsubtract",
    "srvcs-floatmultiply",
    "srvcs-floatadd",
];

/// Dependency endpoints, injected as router state so tests can point them at
/// mock services.
#[derive(Clone)]
pub struct Deps {
    pub sum_url: String,
    pub floatdivide_url: String,
    pub floatsubtract_url: String,
    pub floatmultiply_url: String,
    pub floatadd_url: String,
}

#[derive(Serialize, ToSchema)]
pub struct Info {
    pub service: &'static str,
    pub concern: &'static str,
    pub depends_on: Vec<&'static str>,
}

/// `GET /` — service identity (srvcs service standard).
#[utoipa::path(get, path = "/", responses((status = 200, body = Info)))]
pub async fn index() -> Json<Info> {
    Json(Info {
        service: SERVICE,
        concern: CONCERN,
        depends_on: DEPENDS_ON.to_vec(),
    })
}

#[derive(Deserialize, ToSchema)]
pub struct EvalRequest {
    /// The first list of numbers. Must be non-empty and the same length as `b`.
    #[schema(value_type = Object)]
    pub a: Vec<Value>,
    /// The second list of numbers. Must be non-empty and the same length as `a`.
    #[schema(value_type = Object)]
    pub b: Vec<Value>,
}

#[derive(Serialize, ToSchema)]
pub struct CovarianceResponse {
    #[schema(value_type = Object)]
    pub a: Vec<Value>,
    #[schema(value_type = Object)]
    pub b: Vec<Value>,
    pub result: f64,
}

fn degraded(dependency: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "dependency unavailable", "dependency": dependency })),
    )
        .into_response()
}

fn forward(status: u16, body: Value) -> Response {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    (code, Json(body)).into_response()
}

/// A reachable dependency answered `200` but its body lacked a numeric
/// `result`. That is a contract violation we cannot recover from, so surface a
/// `500` rather than guessing.
fn malformed(dependency: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(
            json!({ "error": "dependency returned a malformed result", "dependency": dependency }),
        ),
    )
        .into_response()
}

/// Ask the integer `srvcs-sum` dependency to total `body`, returning its `i64`
/// `result` or an early-return `Response` the caller should surface verbatim:
///
/// - unreachable / non-`200`/`422` -> `503` degraded
/// - `422` -> forwarded `422` (the dependency rejected an input)
/// - `200` without an `i64` `result` -> `500` malformed
async fn ask_i64(url: &str, body: &Value, dependency: &str) -> Result<i64, Response> {
    match client::call(url, body).await {
        Err(DepError::Unreachable) => Err(degraded(dependency)),
        Ok((200, body)) => match body.get("result").and_then(Value::as_i64) {
            Some(result) => Ok(result),
            None => Err(malformed(dependency)),
        },
        Ok((422, body)) => Err(forward(422, body)),
        Ok(_) => Err(degraded(dependency)),
    }
}

/// Call one float dependency at `url` with `body`, returning its numeric
/// `result` or an early-return `Response` the caller should surface verbatim:
///
/// - unreachable / non-`200`/`422` -> `503` degraded
/// - `422` -> forwarded `422` (the dependency rejected an input)
/// - `200` without an `f64` `result` -> `500` malformed
async fn ask_f64(url: &str, body: &Value, dependency: &str) -> Result<f64, Response> {
    match client::call(url, body).await {
        Err(DepError::Unreachable) => Err(degraded(dependency)),
        Ok((200, body)) => match body.get("result").and_then(Value::as_f64) {
            Some(result) => Ok(result),
            None => Err(malformed(dependency)),
        },
        Ok((422, body)) => Err(forward(422, body)),
        Ok(_) => Err(degraded(dependency)),
    }
}

/// `POST /` — compute the population covariance of two equal-length lists.
///
/// This service owns the *control flow* but delegates every arithmetic step to
/// its dependencies, exactly as specified:
///
/// 1. `meanA = floatdivide(sum(a), n)` and `meanB = floatdivide(sum(b), n)`;
/// 2. fold over each `i`: `da = floatsubtract(a[i], meanA)`,
///    `db = floatsubtract(b[i], meanB)`, `p = floatmultiply(da, db)`,
///    `s = floatadd(s, p)` (starting from `0.0`);
/// 3. `result = floatdivide(s, n)`.
///
/// `a` and `b` must be non-empty and of equal length, otherwise the request is
/// rejected with `422` before any dependency call. If a dependency is
/// unreachable it reports itself degraded (`503`); if a dependency rejects an
/// input it forwards the `422`.
#[utoipa::path(
    post,
    path = "/",
    request_body = EvalRequest,
    responses(
        (status = 200, body = CovarianceResponse),
        (status = 422, description = "invalid input, or a dependency rejected an input (forwarded)"),
        (status = 500, description = "a dependency returned a malformed result"),
        (status = 503, description = "a dependency is unavailable")
    )
)]
pub async fn evaluate(State(deps): State<Deps>, Json(req): Json<EvalRequest>) -> Response {
    if req.a.is_empty() || req.a.len() != req.b.len() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "a and b must be non-empty and equal length" })),
        )
            .into_response();
    }

    let n = req.a.len();

    // 1. meanA = sum(a) / n; meanB = sum(b) / n.
    let sum_a = match ask_i64(&deps.sum_url, &json!({ "values": req.a }), "srvcs-sum").await {
        Ok(t) => t,
        Err(resp) => return resp,
    };
    let mean_a = match ask_f64(
        &deps.floatdivide_url,
        &json!({ "a": sum_a, "b": n }),
        "srvcs-floatdivide",
    )
    .await
    {
        Ok(m) => m,
        Err(resp) => return resp,
    };

    let sum_b = match ask_i64(&deps.sum_url, &json!({ "values": req.b }), "srvcs-sum").await {
        Ok(t) => t,
        Err(resp) => return resp,
    };
    let mean_b = match ask_f64(
        &deps.floatdivide_url,
        &json!({ "a": sum_b, "b": n }),
        "srvcs-floatdivide",
    )
    .await
    {
        Ok(m) => m,
        Err(resp) => return resp,
    };

    // 2. s = fold floatadd over (a[i] - meanA) * (b[i] - meanB), starting 0.0.
    let mut s: f64 = 0.0;
    for (a_i, b_i) in req.a.iter().zip(req.b.iter()) {
        let da = match ask_f64(
            &deps.floatsubtract_url,
            &json!({ "a": a_i, "b": mean_a }),
            "srvcs-floatsubtract",
        )
        .await
        {
            Ok(d) => d,
            Err(resp) => return resp,
        };
        let db = match ask_f64(
            &deps.floatsubtract_url,
            &json!({ "a": b_i, "b": mean_b }),
            "srvcs-floatsubtract",
        )
        .await
        {
            Ok(d) => d,
            Err(resp) => return resp,
        };
        let p = match ask_f64(
            &deps.floatmultiply_url,
            &json!({ "a": da, "b": db }),
            "srvcs-floatmultiply",
        )
        .await
        {
            Ok(p) => p,
            Err(resp) => return resp,
        };
        s = match ask_f64(
            &deps.floatadd_url,
            &json!({ "a": s, "b": p }),
            "srvcs-floatadd",
        )
        .await
        {
            Ok(s) => s,
            Err(resp) => return resp,
        };
    }

    // 3. result = s / n.
    let result = match ask_f64(
        &deps.floatdivide_url,
        &json!({ "a": s, "b": n }),
        "srvcs-floatdivide",
    )
    .await
    {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    (
        StatusCode::OK,
        Json(json!({ "a": req.a, "b": req.b, "result": result })),
    )
        .into_response()
}

#[derive(OpenApi)]
#[openapi(
    paths(index, evaluate),
    components(schemas(Info, EvalRequest, CovarianceResponse))
)]
pub struct ApiDoc;

/// Serve OpenAPI document
pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_documents_routes() {
        let doc = ApiDoc::openapi();
        let root = doc.paths.paths.get("/").expect("path / present");
        assert!(root.get.is_some());
        assert!(root.post.is_some());
    }

    #[tokio::test]
    async fn index_reports_all_dependencies() {
        let Json(info) = index().await;
        assert_eq!(info.service, "srvcs-covariance");
        assert_eq!(info.concern, "statistics: population covariance");
        assert_eq!(
            info.depends_on,
            vec![
                "srvcs-sum",
                "srvcs-floatdivide",
                "srvcs-floatsubtract",
                "srvcs-floatmultiply",
                "srvcs-floatadd"
            ]
        );
    }
}
