use axum::body::Body;
use axum::extract::Json as JsonExtract;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router as AxumRouter};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use srvcs_covariance::{api::Deps, health, router, telemetry};
use tower::ServiceExt;

const DEAD_URL: &str = "http://127.0.0.1:1";

async fn serve(app: AxumRouter) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// Read an `f64` operand named `key`, defaulting to `default` when absent.
fn num(body: &Value, key: &str, default: f64) -> f64 {
    body.get(key).and_then(Value::as_f64).unwrap_or(default)
}

/// Mock `srvcs-sum` that ACTUALLY COMPUTES the integer sum of the `values`
/// array and returns `{"values", "result": <i64>}`.
async fn spawn_computing_sum() -> String {
    let app = AxumRouter::new().route(
        "/",
        post(|JsonExtract(req): JsonExtract<Value>| async move {
            let total: i64 = req["values"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_i64).sum())
                .unwrap_or(0);
            Json(json!({ "values": req["values"], "result": total }))
        }),
    );
    serve(app).await
}

/// Mock `srvcs-floatadd` that ACTUALLY COMPUTES `a + b` as an `f64`.
async fn spawn_computing_floatadd() -> String {
    let app = AxumRouter::new().route(
        "/",
        post(|JsonExtract(body): JsonExtract<Value>| async move {
            Json(json!({ "result": num(&body, "a", 0.0) + num(&body, "b", 0.0) }))
        }),
    );
    serve(app).await
}

/// Mock `srvcs-floatsubtract` that ACTUALLY COMPUTES `a - b` as an `f64`.
async fn spawn_computing_floatsubtract() -> String {
    let app = AxumRouter::new().route(
        "/",
        post(|JsonExtract(body): JsonExtract<Value>| async move {
            Json(json!({ "result": num(&body, "a", 0.0) - num(&body, "b", 0.0) }))
        }),
    );
    serve(app).await
}

/// Mock `srvcs-floatmultiply` that ACTUALLY COMPUTES `a * b` as an `f64`.
async fn spawn_computing_floatmultiply() -> String {
    let app = AxumRouter::new().route(
        "/",
        post(|JsonExtract(body): JsonExtract<Value>| async move {
            Json(json!({ "result": num(&body, "a", 0.0) * num(&body, "b", 0.0) }))
        }),
    );
    serve(app).await
}

/// Mock `srvcs-floatdivide` that ACTUALLY COMPUTES `a / b` as an `f64`, or
/// `422` on divide-by-zero.
async fn spawn_computing_floatdivide() -> String {
    let app = AxumRouter::new().route(
        "/",
        post(|JsonExtract(body): JsonExtract<Value>| async move {
            let b = num(&body, "b", 1.0);
            if b == 0.0 {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({ "error": "divide by zero" })),
                );
            }
            (
                StatusCode::OK,
                Json(json!({ "result": num(&body, "a", 0.0) / b })),
            )
        }),
    );
    serve(app).await
}

/// Mock that always answers with a fixed status + body (error-path tests).
async fn spawn_fixed(status: StatusCode, body: Value) -> String {
    let app = AxumRouter::new().route(
        "/",
        post(move || {
            let body = body.clone();
            async move { (status, Json(body)) }
        }),
    );
    serve(app).await
}

#[derive(Clone)]
struct Urls {
    sum: String,
    floatdivide: String,
    floatsubtract: String,
    floatmultiply: String,
    floatadd: String,
}

/// All dependencies are computing mocks: exercises the genuine composition.
async fn all_computing() -> Urls {
    Urls {
        sum: spawn_computing_sum().await,
        floatdivide: spawn_computing_floatdivide().await,
        floatsubtract: spawn_computing_floatsubtract().await,
        floatmultiply: spawn_computing_floatmultiply().await,
        floatadd: spawn_computing_floatadd().await,
    }
}

fn dead_urls() -> Urls {
    Urls {
        sum: DEAD_URL.to_string(),
        floatdivide: DEAD_URL.to_string(),
        floatsubtract: DEAD_URL.to_string(),
        floatmultiply: DEAD_URL.to_string(),
        floatadd: DEAD_URL.to_string(),
    }
}

fn app(u: &Urls) -> axum::Router {
    router(
        telemetry::metrics_handle_for_tests(),
        Deps {
            sum_url: u.sum.clone(),
            floatdivide_url: u.floatdivide.clone(),
            floatsubtract_url: u.floatsubtract.clone(),
            floatmultiply_url: u.floatmultiply.clone(),
            floatadd_url: u.floatadd.clone(),
        },
    )
}

async fn eval(u: &Urls, a: Value, b: Value) -> (StatusCode, Value) {
    let res = app(u)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "a": a, "b": b }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn status_of(uri: &str) -> StatusCode {
    app(&dead_urls())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

/// Approximate float comparison: floats must never be compared for exact
/// equality.
fn approx(got: &Value, expected: f64) -> bool {
    got.as_f64().map(|x| (x - expected).abs() < 1e-9) == Some(true)
}

// --- Standard endpoints ---

#[tokio::test]
async fn healthz_ok() {
    assert_eq!(status_of("/healthz").await, StatusCode::OK);
}

#[tokio::test]
async fn readyz_reflects_state() {
    health::set_ready(true);
    assert_eq!(status_of("/readyz").await, StatusCode::OK);
}

#[tokio::test]
async fn metrics_ok() {
    assert_eq!(status_of("/metrics").await, StatusCode::OK);
}

#[tokio::test]
async fn openapi_ok() {
    assert_eq!(status_of("/openapi.json").await, StatusCode::OK);
}

#[tokio::test]
async fn generates_request_id_when_absent() {
    let res = app(&dead_urls())
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        res.headers().contains_key("x-request-id"),
        "response must carry a generated x-request-id"
    );
}

#[tokio::test]
async fn index_reports_identity() {
    let res = app(&dead_urls())
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["service"], "srvcs-covariance");
    assert_eq!(body["concern"], "statistics: population covariance");
    assert_eq!(
        body["depends_on"],
        json!([
            "srvcs-sum",
            "srvcs-floatdivide",
            "srvcs-floatsubtract",
            "srvcs-floatmultiply",
            "srvcs-floatadd"
        ])
    );
}

// --- Correctness cases, exercised against REAL computing dependencies ---

#[tokio::test]
async fn covariance_of_identical_lists_is_variance() {
    let u = all_computing().await;
    // means 2, 2; deviations [-1, 0, 1] each; products [1, 0, 1]; s = 2; / 3
    let (status, body) = eval(&u, json!([1, 2, 3]), json!([1, 2, 3])).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        approx(&body["result"], 0.6666666666666666),
        "got {:?}",
        body["result"]
    );
    assert_eq!(body["a"], json!([1, 2, 3]));
    assert_eq!(body["b"], json!([1, 2, 3]));
}

#[tokio::test]
async fn covariance_is_negative_for_anticorrelated_lists() {
    let u = all_computing().await;
    // a means 2, b means 2; a dev [-1,0,1], b dev [1,0,-1]; products [-1,0,-1]
    // s = -2; / 3 = -0.6666...
    let (status, body) = eval(&u, json!([1, 2, 3]), json!([3, 2, 1])).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        approx(&body["result"], -0.6666666666666666),
        "got {:?}",
        body["result"]
    );
}

#[tokio::test]
async fn covariance_with_constant_list_is_zero() {
    let u = all_computing().await;
    // b is constant -> every db is 0 -> covariance 0
    let (status, body) = eval(&u, json!([1, 2, 3, 4]), json!([5, 5, 5, 5])).await;
    assert_eq!(status, StatusCode::OK);
    assert!(approx(&body["result"], 0.0), "got {:?}", body["result"]);
}

#[tokio::test]
async fn covariance_of_singletons_is_zero() {
    let u = all_computing().await;
    // n = 1: each deviation is 0 -> covariance 0
    let (status, body) = eval(&u, json!([7]), json!([4])).await;
    assert_eq!(status, StatusCode::OK);
    assert!(approx(&body["result"], 0.0), "got {:?}", body["result"]);
}

#[tokio::test]
async fn covariance_scaled() {
    let u = all_computing().await;
    // a [1,2,3] mean 2 dev [-1,0,1]; b [2,4,6] mean 4 dev [-2,0,2]
    // products [2,0,2]; s = 4; / 3 = 1.3333...
    let (status, body) = eval(&u, json!([1, 2, 3]), json!([2, 4, 6])).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        approx(&body["result"], 4.0 / 3.0),
        "got {:?}",
        body["result"]
    );
}

#[tokio::test]
async fn covariance_with_negatives() {
    let u = all_computing().await;
    // a [-2,0,2] mean 0 dev [-2,0,2]; b [1,2,3] mean 2 dev [-1,0,1]
    // products [2,0,2]; s = 4; / 3 = 1.3333...
    let (status, body) = eval(&u, json!([-2, 0, 2]), json!([1, 2, 3])).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        approx(&body["result"], 4.0 / 3.0),
        "got {:?}",
        body["result"]
    );
}

// --- Validation / error / degraded paths ---

#[tokio::test]
async fn empty_lists_is_422_with_no_calls() {
    // DEAD_URLs: if it tried to call any dependency it would degrade to 503.
    let (status, body) = eval(&dead_urls(), json!([]), json!([])).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "a and b must be non-empty and equal length");
}

#[tokio::test]
async fn mismatched_lengths_is_422_with_no_calls() {
    let (status, body) = eval(&dead_urls(), json!([1, 2, 3]), json!([1, 2])).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "a and b must be non-empty and equal length");
}

#[tokio::test]
async fn degrades_when_sum_unreachable() {
    let mut u = all_computing().await;
    u.sum = DEAD_URL.to_string();
    let (status, body) = eval(&u, json!([1, 2, 3]), json!([1, 2, 3])).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["dependency"], "srvcs-sum");
}

#[tokio::test]
async fn degrades_when_floatdivide_unreachable() {
    let mut u = all_computing().await;
    u.floatdivide = DEAD_URL.to_string();
    let (status, body) = eval(&u, json!([1, 2, 3]), json!([1, 2, 3])).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["dependency"], "srvcs-floatdivide");
}

#[tokio::test]
async fn degrades_when_floatsubtract_unreachable() {
    let mut u = all_computing().await;
    u.floatsubtract = DEAD_URL.to_string();
    let (status, body) = eval(&u, json!([1, 2, 3]), json!([1, 2, 3])).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["dependency"], "srvcs-floatsubtract");
}

#[tokio::test]
async fn degrades_when_floatmultiply_unreachable() {
    let mut u = all_computing().await;
    u.floatmultiply = DEAD_URL.to_string();
    let (status, body) = eval(&u, json!([1, 2, 3]), json!([1, 2, 3])).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["dependency"], "srvcs-floatmultiply");
}

#[tokio::test]
async fn degrades_when_floatadd_unreachable() {
    let mut u = all_computing().await;
    u.floatadd = DEAD_URL.to_string();
    let (status, body) = eval(&u, json!([1, 2, 3]), json!([1, 2, 3])).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["dependency"], "srvcs-floatadd");
}

#[tokio::test]
async fn forwards_422_from_sum() {
    let mut u = all_computing().await;
    u.sum = spawn_fixed(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({ "error": "value is not a number" }),
    )
    .await;
    let (status, body) = eval(&u, json!([1, "nope", 3]), json!([1, 2, 3])).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "value is not a number");
}

#[tokio::test]
async fn forwards_422_from_floatsubtract() {
    let mut u = all_computing().await;
    u.floatsubtract = spawn_fixed(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({ "error": "value is not a number" }),
    )
    .await;
    let (status, _) = eval(&u, json!([1, 2, 3]), json!([1, 2, 3])).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn malformed_sum_result_is_500() {
    let mut u = all_computing().await;
    u.sum = spawn_fixed(StatusCode::OK, json!({ "result": "not-a-number" })).await;
    let (status, body) = eval(&u, json!([1, 2, 3]), json!([1, 2, 3])).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["dependency"], "srvcs-sum");
}

#[tokio::test]
async fn malformed_floatadd_result_is_500() {
    let mut u = all_computing().await;
    u.floatadd = spawn_fixed(StatusCode::OK, json!({ "result": "not-a-number" })).await;
    let (status, body) = eval(&u, json!([1, 2, 3]), json!([1, 2, 3])).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["dependency"], "srvcs-floatadd");
}
