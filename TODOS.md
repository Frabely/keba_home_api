# TODOS

## 1-2 Tage Testrun
- [ ] Service als Dauerlauf stabil betreiben (keine unnötigen Neustarts).
- [ ] SQLite als primäre Datenquelle nutzen (`sessions` bleibt Source of truth).
- [ ] `results.json` nur als Zusatzexport behalten (optional).
- [ ] Vor Teststart Config-Snapshot dokumentieren (Quelle, Debounce, Poll-Intervall, Bind, DB-Pfad).
- [ ] Nach Testzeitraum DB + results + relevante Logs sichern.
- [ ] Auswertung durchführen:
  - Anzahl Sessions
  - Verteilung `durationMs` und `kwh`
  - Anomalien: 0-kWh Sessions, sehr kurze Sessions, unerwartete Lücken
  - Plausibilitätscheck gegen 2-3 manuell beobachtete Ladevorgänge

## Logging in SQL (Vorschlag)

### Ziel
Sinnvolle Events (insb. Fehler) strukturiert in DB speichern, damit sie für Auswertung und Debugging abfragbar sind.

### Event-Tabelle
```sql
CREATE TABLE IF NOT EXISTS log_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    occurred_at TEXT NOT NULL,            -- UTC ISO-8601
    level TEXT NOT NULL,                  -- INFO | WARN | ERROR
    component TEXT NOT NULL,              -- z.B. app.runtime, adapters.keba_udp
    event_code TEXT NOT NULL,             -- stabile Code-ID, z.B. POLL_FETCH_R2_TIMEOUT
    message TEXT NOT NULL,                -- menschenlesbare Kurzbeschreibung

    source TEXT,                          -- udp | modbus | debug_file
    station TEXT,                         -- optional: Carport/Eingang
    session_id INTEGER,                   -- optionaler Bezug zu sessions.id

    error_code TEXT,                      -- optional: domänenspez. Code / OS code als String
    error_kind TEXT,                      -- z.B. timed_out, network_unreachable
    error_detail TEXT,                    -- detailierter Fehlertext

    context_json TEXT                     -- zusätzliche strukturierte Daten als JSON-String
);

CREATE INDEX IF NOT EXISTS idx_log_events_occurred_at_desc
ON log_events (occurred_at DESC);

CREATE INDEX IF NOT EXISTS idx_log_events_level_occurred_at_desc
ON log_events (level, occurred_at DESC);

CREATE INDEX IF NOT EXISTS idx_log_events_event_code_occurred_at_desc
ON log_events (event_code, occurred_at DESC);

CREATE INDEX IF NOT EXISTS idx_log_events_session_id
ON log_events (session_id);
```

### Event-Codes (Startmenge)
- `SERVICE_START`
- `SERVICE_STOP`
- `POLL_CYCLE_FAILED`
- `POLL_FETCH_REPORT2_FAILED`
- `POLL_FETCH_REPORT3_FAILED`
- `POLL_PARSE_REPORT2_FAILED`
- `POLL_PARSE_REPORT3_FAILED`
- `SESSION_STARTED`
- `SESSION_PERSISTED`
- `SESSION_PERSIST_FAILED`
- `RESULTS_JSON_WRITE_FAILED`
- `DEBUG_REPLAY_FINISHED`

### Logging-Strategie
- INFO nur für Lifecycle + Session-Events.
- WARN/ERROR für echte Probleme (Netzwerk/Parse/DB/Write).
- Poll-Schleife nicht pro Tick auf INFO loggen (zu viel Rauschen).
- Wiederholte identische Fehler optional deduplizieren oder mit Counter aggregieren.

## Umsetzung (nächste Schritte)
- [ ] Migration für `log_events` hinzufügen.
- [ ] `insert_log_event(...)` im DB-Adapter ergänzen.
- [ ] Runtime/Adapter um gezielte Event-Codes erweitern.
- [ ] Optional Endpoint hinzufügen: `/diagnostics/logs?level=&eventCode=&limit=&offset=`
