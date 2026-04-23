# API Contract

All timestamps are UTC ISO-8601 (`...Z`) unless explicitly documented otherwise.
JSON fields use `camelCase`, except passthrough KEBA compatibility fields (`kWh`, `CardId`) and `/unplug-log` response fields (`Id`, `Timestamp`, `Station`, `Started`, `Ended`, `Wh`, `CardId`).
The canonical endpoints live under `/api/v1`.
The old root paths without `/api/v1` remain reachable for now for compatibility.
Browser access is controlled via `CORS_ALLOWED_ORIGINS`; defaults are `http://localhost:3000` and `https://invessiv.de`.

Without a domain, the host is simply:
```text
http://<PUBLIC_IP>:65109
```

Example:
```text
http://84.123.45.67:65109/api/v1/health
```

## Auth

There is intentionally no authentication at the moment. All read-only endpoints are reachable without a token.
If the API is exposed to the public internet, access is therefore currently unencrypted and without request auth.

## `GET /api/v1/health`
Health check endpoint.

Example:
```bash
curl -s http://localhost:65109/api/v1/health | jq
```

Response `200`:
```json
{
  "status": "ok"
}
```

Legacy compatibility path: `GET /health`

## `GET /api/v1/sessions/carport/latest`
Fetch latest session view from KEBA `report 100..130`.  
The API takes the first report where `started > 0`, `ended > 0` and `E Pres >= 0`.

Example:
```bash
curl -s http://localhost:65109/api/v1/sessions/carport/latest | jq
```

Response `200`:
```json
{
  "reportId": 103,
  "kWh": 7.65,
  "started": 1772386819000,
  "ended": 1772427719000,
  "CardId": "XYZ999"
}
```

Legacy compatibility path: `GET /sessions/carport/latest`

## `GET /api/v1/sessions/entrance/latest`
Same contract as `/sessions/carport/latest`, but for station `entrance`.

Example:
```bash
curl -s http://localhost:65109/api/v1/sessions/entrance/latest | jq
```

Response `200`: same JSON shape as above.

Legacy compatibility path: `GET /sessions/entrance/latest`

## `GET /api/v1/dachs/f233/status`
Reads Dachs operating data via the configured HTTP upstream `DACHS_F233_BASE_URL/getKey` and maps the requested keys to stable API fields.
Default credentials are `GLT` / `HECHT`.
If at least one of `DACHS_F233_USERNAME` or `DACHS_F233_PASSWORD` is set, the upstream request attempts HTTP Basic Auth. If both values are missing, the request is sent without auth.

Example:
```bash
curl -s http://localhost:65109/api/v1/dachs/f233/status | jq
```

Response `200`:
```json
{
  "starts": 476,
  "bh": 47088.02,
  "electricityInternal": 12345.678,
  "heat": 98765.432,
  "maintenance": 3099.496,
  "buderusStarts": 91,
  "buderusBh": 2468.1
}
```

Mapping:
- `starts` <= `Hka_Bd.ulAnzahlStarts`
- `bh` <= `Hka_Bd.ulBetriebssekunden`
- `electricityInternal` <= `Hka_Bd.ulArbeitElektr`
- `heat` <= `Hka_Bd.ulArbeitThermHka`
- `maintenance` <= `3500 - (Hka_Bd.ulBetriebssekunden - Wartung_Cache.ulBetriebssekundenBei)`
- `buderusStarts` <= `Brenner_Bd.ulAnzahlStarts`
- `buderusBh` <= `Brenner_Bd.ulBetriebssekunden`

## `GET /api/v1/dachs/f235/status`
Same contract as `/api/v1/dachs/f233/status`, but uses `DACHS_F235_BASE_URL`, `DACHS_F235_USERNAME` and `DACHS_F235_PASSWORD`.
This endpoint does not read or return `Brenner_Bd.*` fields, so the response only contains:
`starts`, `bh`, `electricityInternal`, `heat`, `maintenance`.
If at least one of `DACHS_F235_USERNAME` or `DACHS_F235_PASSWORD` is set, the upstream request attempts HTTP Basic Auth. If both values are missing, the request is sent without auth.

Example:
```bash
curl -s http://localhost:65109/api/v1/dachs/f235/status | jq
```

Default upstream for this endpoint: `http://192.168.233.92:8080`.

Response `200`:
```json
{
  "starts": 12,
  "bh": 1234.5,
  "electricityInternal": 2500.0,
  "heat": 3750.5,
  "maintenance": 3225.0
}
```

## `GET /api/v1/unplug-log?count={x}`
Returns the latest entries from `unplug_log_events`, sorted by `Timestamp DESC, Id DESC`.
`count` corresponds to a SQL `LIMIT` (similar to `SELECT TOP x ...`) and is optional.

Examples:
```bash
curl -s "http://localhost:65109/api/v1/unplug-log?count=5" | jq
curl -s "http://localhost:65109/api/v1/unplug-log" | jq
```

Response `200`:
```json
[
  {
    "Id": "c8d9b95b-6d73-4f0f-8a51-2dbd1f9f57d8",
    "Timestamp": "2026-03-04 11:00",
    "Station": "Carport",
    "Started": "n/a",
    "Ended": "n/a",
    "Wh": "0.0",
    "CardId": "CARD-3"
  }
]
```

## Error Responses

`404` (station mapping missing):
```json
{
  "error": "station mapping for 'entrance' is not configured"
}
```

`400` (invalid `count` parameter):
```json
{
  "error": "query parameter 'count' must be >= 1"
}
```

`502` (KEBA communication/payload issue):
```json
{
  "error": "failed to fetch report 100: transport communication failed: ..."
}
```

or

```json
{
  "error": "reports 100-130 do not contain started/end timestamps and E Pres >= 0"
}
```

`503` (Dachs endpoint disabled):
```json
{
  "error": "dachs f233 status endpoint is not configured"
}
```

`502` (Dachs upstream/payload issue):
```json
{
  "error": "failed to fetch dachs status: dachs upstream request failed: ..."
}
```

`404` (removed legacy path):
```json
{
  "error": "not found"
}
```

## CORS

Default:
```bash
CORS_ALLOWED_ORIGINS=http://localhost:3000
```

Restricted operation with a known frontend origin:
```bash
CORS_ALLOWED_ORIGINS=http://localhost:3000,https://invessiv.de
```

The API answers CORS preflights directly via the Actix CORS middleware and allows at least `GET` and `OPTIONS` as well as the headers `Accept`, `Authorization`, and `Content-Type`.
