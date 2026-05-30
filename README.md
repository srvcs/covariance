# srvcs-covariance

Statistics microservice for srvcs.cloud: computes the **population covariance** of
two equal-length lists of numbers.

This service is an **orchestrator**. It owns the control flow but delegates every
arithmetic step to its dependencies. It does not validate inputs itself —
validation propagates from its dependencies (their `422`s are forwarded).

## Concern

`statistics: population covariance`

## Dependencies

- `srvcs-sum` — total of a list (integer `result`)
- `srvcs-floatdivide` — `a / b`
- `srvcs-floatsubtract` — `a - b`
- `srvcs-floatmultiply` — `a * b`
- `srvcs-floatadd` — `a + b`

## API

### `GET /`

Service identity.

```json
{
  "service": "srvcs-covariance",
  "concern": "statistics: population covariance",
  "depends_on": [
    "srvcs-sum",
    "srvcs-floatdivide",
    "srvcs-floatsubtract",
    "srvcs-floatmultiply",
    "srvcs-floatadd"
  ]
}
```

### `POST /`

Request:

```json
{ "a": [1, 2, 3], "b": [1, 2, 3] }
```

Response `200`:

```json
{ "a": [1, 2, 3], "b": [1, 2, 3], "result": 0.6666666666666666 }
```

## Algorithm

Given lists `a` and `b` with `n = a.len()` (which must equal `b.len()`; empty or
mismatched -> `422`):

1. `meanA = floatdivide(sum(a), n)`
2. `meanB = floatdivide(sum(b), n)`
3. `s = 0.0`; for each `i`:
   - `da = floatsubtract(a[i], meanA)`
   - `db = floatsubtract(b[i], meanB)`
   - `p = floatmultiply(da, db)`
   - `s = floatadd(s, p)`
4. `result = floatdivide(s, n)`

`result` is an `f64`. For example,
`covariance([1, 2, 3], [1, 2, 3]) = 0.6666666666666666`.

## Status codes

- `200` — computed covariance
- `422` — empty/mismatched lists, or a dependency rejected an input (forwarded)
- `500` — a dependency returned a malformed result
- `503` — a dependency is unavailable

## Configuration

| Variable                  | Default                  | Description                        |
| ------------------------- | ------------------------ | ---------------------------------- |
| `SRVCS_BIND_ADDR`         | `0.0.0.0:8080`           | Listen address (`host:port`).      |
| `SRVCS_SUM_URL`           | `http://127.0.0.1:8090`  | Base URL of `srvcs-sum`.           |
| `SRVCS_FLOATDIVIDE_URL`   | `http://127.0.0.1:8091`  | Base URL of `srvcs-floatdivide`.   |
| `SRVCS_FLOATSUBTRACT_URL` | `http://127.0.0.1:8092`  | Base URL of `srvcs-floatsubtract`. |
| `SRVCS_FLOATMULTIPLY_URL` | `http://127.0.0.1:8093`  | Base URL of `srvcs-floatmultiply`. |
| `SRVCS_FLOATADD_URL`      | `http://127.0.0.1:8094`  | Base URL of `srvcs-floatadd`.      |
| `RUST_LOG`                | `info,tower_http=info`   | Log filter.                        |
| `SRVCS_ENV`               | `development`            | Environment label.                 |
