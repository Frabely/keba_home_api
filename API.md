# API Contract

All timestamps are UTC ISO-8601 (`...Z`) unless explicitly documented otherwise.
JSON fields use `camelCase`, except passthrough KEBA compatibility fields (`kWh`, `CardId`) and `/unplug-log` response fields (`Id`, `Timestamp`, `Station`, `Started`, `Ended`, `Wh`, `CardId`).
Die kanonischen Endpunkte liegen unter `/api/v1`.
Die bisherigen Root-Pfade ohne `/api/v1` bleiben vorerst aus Kompatibilitaetsgruenden erreichbar.
Browser-Zugriffe werden ueber `CORS_ALLOWED_ORIGINS` gesteuert; Default sind `http://localhost:3000` und `https://invessiv.de`.

Ohne Domain lautet der Host einfach:
```text
http://<OEFFENTLICHE_IP>:65109
```

Beispiel:
```text
http://84.123.45.67:65109/api/v1/health
```

## Auth

Aktuell gibt es bewusst keine Authentifizierung. Alle Readonly-Endpunkte sind ohne Token erreichbar.
Wenn die API oeffentlich ins Internet gestellt wird, erfolgt der Zugriff derzeit also unverschluesselt und ohne Request-Auth.

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

## `GET /api/v1/dachs/status`
Liest Dachs-Betriebsdaten ueber den konfigurierten HTTP-Upstream `DACHS_BASE_URL/getKey` und mapped die angeforderten Keys auf stabile API-Felder.
Wenn gesetzt, werden `DACHS_USERNAME` und `DACHS_PASSWORD` direkt in die Upstream-URL eingebettet, also im Stil `http://user:pass@host/getKey?...`.

Example:
```bash
curl -s http://localhost:65109/api/v1/dachs/status | jq
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

Legacy compatibility path: `GET /dachs/status`

## `GET /api/v1/unplug-log?count={x}`
Liefert die neuesten Eintraege aus `unplug_log_events`, sortiert nach `Timestamp DESC, Id DESC`.
`count` entspricht einem SQL-`LIMIT` (vergleichbar mit `SELECT TOP x ...`) und ist optional.

Beispiele:
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

`503` (Dachs endpoint disabled):
```json
{
  "error": "dachs status endpoint is not configured"
}
```

`502` (Dachs upstream/payload issue):
```json
{
  "error": "failed to fetch dachs status: dachs upstream request failed: ..."
}
```

## CORS

Default:
```bash
CORS_ALLOWED_ORIGINS=http://localhost:3000
```

Restriktiver Betrieb mit bekannter Frontend-Origin:
```bash
CORS_ALLOWED_ORIGINS=http://localhost:3000,https://invessiv.de
```

Die API beantwortet CORS-Preflights direkt ueber die Actix-CORS-Middleware und erlaubt dabei mindestens `GET` und `OPTIONS` sowie die Header `Accept`, `Authorization` und `Content-Type`.
