# API Contract

All timestamps are UTC ISO-8601 (`...Z`) unless explicitly documented otherwise.
JSON fields use `camelCase`, except passthrough KEBA compatibility fields (`kWh`, `CardId`) and `/unplug-log` response fields (`Id`, `Timestamp`, `Station`, `Started`, `Ended`, `Wh`, `CardId`).
Session endpoints sind aktuell ohne API-Key erreichbar.

## `GET /health`
Health check endpoint.

Example:
```bash
curl -s http://localhost:8080/health | jq
```

Response `200`:
```json
{
  "status": "ok"
}
```

## `GET /sessions/carport/latest`
Fetch latest session view from KEBA `report 100..130`.  
The API takes the first report where `started > 0`, `ended > 0` and `E Pres >= 0`.

Example:
```bash
curl -s http://localhost:8080/sessions/carport/latest | jq
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

## `GET /sessions/entrance/latest`
Same contract as `/sessions/carport/latest`, but for station `entrance`.

Example:
```bash
curl -s http://localhost:8080/sessions/entrance/latest | jq
```

Response `200`: same JSON shape as above.

## `GET /unplug-log?count={x}`
Liefert die neuesten Eintraege aus `unplug_log_events`, sortiert nach `Timestamp DESC, Id DESC`.
`count` entspricht einem SQL-`LIMIT` (vergleichbar mit `SELECT TOP x ...`) und ist optional.

Beispiele:
```bash
curl -s "http://localhost:8080/unplug-log?count=5" | jq
curl -s "http://localhost:8080/unplug-log" | jq
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

`400` (ungueltiger `count` Parameter):
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
