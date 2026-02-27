# API Documentation

Base URL (default): `http://localhost:8080`

All timestamps are UTC ISO-8601 (`...Z`), JSON fields are `camelCase`.

## Quick Check

```bash
curl -s http://localhost:8080/health | jq
```

Expected:

```json
{ "status": "ok" }
```

## Endpoints

### 1) Health

`GET /health`

Purpose: service liveness probe.

Example:

```bash
curl -s http://localhost:8080/health | jq
```

### 2) Latest Session

`GET /sessions/latest`

Purpose: returns the latest persisted charging session.

Example:

```bash
curl -s http://localhost:8080/sessions/latest | jq
```

Success response (`200`):

```json
{
  "id": "4f7aa40a-f2d6-4e4f-b2d0-414f32f11de1",
  "startedAt": "2026-02-27T14:42:00.000Z",
  "finishedAt": "2026-02-27T14:46:00.000Z",
  "durationMs": 240000,
  "kwh": 4.0
}
```

If none exists (`404`):

```json
{ "error": "no sessions available" }
```

### 3) Session List (Pagination)

`GET /sessions?limit=<1..500>&offset=<0..>`

Purpose: paginated list, newest first.

Example:

```bash
curl -s "http://localhost:8080/sessions?limit=20&offset=0" | jq
```

### 4) Recent Session (last 5 minutes)

`GET /sessions/recent`

Purpose: returns latest session only if created within the last 5 minutes.

Example:

```bash
curl -i -s http://localhost:8080/sessions/recent
```

Responses:
- `200` with session body (same shape as `/sessions/latest`)
- `204 No Content` if none in last 5 minutes

### 5) DB Diagnostics

`GET /diagnostics/db`

Purpose: runtime DB state for diagnostics.

Example:

```bash
curl -s http://localhost:8080/diagnostics/db | jq
```

Response (`200`):

```json
{
  "schemaVersion": 4,
  "sessionsCount": 12,
  "logEventsCount": 37,
  "latestSession": {
    "id": "0b06eb56-e000-4f66-83e2-b6324afe6f12",
    "status": "completed",
    "startedReason": "plug_state_transition",
    "finishedReason": "plug_state_transition",
    "startedAt": "2026-02-27T14:42:00.000Z",
    "finishedAt": "2026-02-27T14:46:00.000Z",
    "durationMs": 240000,
    "kwh": 4.0,
    "errorCountDuringSession": 1
  }
}
```

### 6) Recent Log Events Diagnostics

`GET /diagnostics/log-events?limit=<1..500>`

Purpose: inspect latest persisted poll/service log events.

Example:

```bash
curl -s "http://localhost:8080/diagnostics/log-events?limit=20" | jq
```

Response (`200`) example item:

```json
{
  "id": "b4b95a7f-cf4f-4ca8-a363-5f4a7f95b9f9",
  "createdAt": "2026-02-27T22:53:46.574Z",
  "level": "warn",
  "code": "poll.fetch_report2",
  "message": "failed to fetch report 2: transport communication failed: ...",
  "source": "udp",
  "stationId": "carport",
  "detailsJson": "{\"activeSession\":true,\"errorCountDuringSession\":3}"
}
```

## Error Model

Current error payload shape:

```json
{ "error": "<message>" }
```

Typical statuses:
- `200` success
- `204` no content (`/sessions/recent`)
- `404` not found (`/sessions/latest` when empty)
- `500` internal/service/database errors

## Best-Practice Call Set (for smoke checks)

```bash
curl -s http://localhost:8080/health | jq
curl -s http://localhost:8080/diagnostics/db | jq
curl -s "http://localhost:8080/diagnostics/log-events?limit=10" | jq
curl -s http://localhost:8080/sessions/latest | jq
curl -s "http://localhost:8080/sessions?limit=10&offset=0" | jq
curl -i -s http://localhost:8080/sessions/recent
```
