# API Contract

All timestamps are UTC ISO-8601 (`...Z`) unless explicitly documented otherwise.
JSON fields use `camelCase`, except passthrough KEBA compatibility fields (`kWh`, `CardId`).

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
The API takes the first report where `started > 0`, `ended > 0` and `E Pres > 0`.

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

## Error Responses

`404` (station mapping missing):
```json
{
  "error": "station mapping for 'entrance' is not configured"
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
  "error": "reports 100-130 do not contain started/end timestamps and E Pres > 0"
}
```
Hinweis zu `kWh`:
- `E Pres` Werte `>= 1000` werden als `0.1 Wh` interpretiert und in kWh umgerechnet (`/10000`).
- Kleinere Werte werden als bereits in kWh geliefert behandelt.
