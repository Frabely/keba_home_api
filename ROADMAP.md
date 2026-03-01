# roadmap.md — KEBA P30 Home Service (Rust) + SQLite + HTTP API

## Kontext / Ziel (aus der bisherigen Konversation)
- Wallbox: **KEBA KeContact P30** (Kennung u. a. `KC-P30-EC240422-...`)
- Läuft **lokal im Heimnetz** (LAN/WLAN).
- Ziel: Ein **dauerhaft laufender Service** (z. B. auf Raspberry Pi), der:
    1) regelmäßig (ca. **1×/Sekunde**) den Zustand der Wallbox prüft,
    2) **Ladesessions** automatisch erkennt (angesteckt → abgesteckt),
    3) pro Session speichert:
        - **Zeitpunkt angesteckt** (Timestamp vom Service/Host, nicht Wallbox-Uhrzeit)
        - **Zeitpunkt abgesteckt** (Timestamp vom Service/Host)
        - **geladen kWh** (aus Wallbox-Werten)
    4) eine **HTTP API** bereitstellt, damit eine externe App kurz nach dem Laden die letzte Session abholen kann.
- Wichtig: **Firebase Hosting (statisch)** ist nicht geeignet für direkten Zugriff auf Wallbox (UDP/Modbus). Daher läuft die Logik **zuhause**.
- Fokus: **geringer Ressourcenverbrauch** → **Rust** + SQLite + leichtes HTTP-Framework (**actix-web**).
- Wallbox liefert (mindestens):
    - **Seconds Counter** z. B. `plugged : seconds : 4264958` (gut für relative Zeiten, aber echte Uhrzeiten werden vom Service geloggt)
    - **Energy (present session)** z. B. `10,83 kWh`
    - **Energy (total)** z. B. `28193,08 kWh`
- Zusätzlich existieren Logs mit internen Countern und Status (“plugged/unplugged/charging/disabled”), aber für echte Zeitpunkte werden **Host-Timestamps** genutzt.

## Annahmen / Schnittstelle
- KEBA P30 wird idealerweise über **UDP SmartHome Interface** abgefragt (einfacher für Event/Status).
- Alternative wäre Modbus TCP; initial fokussieren wir auf **UDP**.
- UDP liefert JSON-ähnliche Antworten (z. B. `report 2` für Status, `report 3` für Energie/Session).
- Feldnamen können variieren (z. B. `"E pres"` mit Leerzeichen).
- Für robuste Implementierung: Keys konfigurierbar machen oder mehrere Kandidaten unterstützen.

---

## Deliverables (was Codex bauen soll)
1) Rust Service (single binary) mit:
    - UDP Client (KEBA)
    - Polling + State-Machine (Session-Erkennung)
    - SQLite Persistenz
    - actix-web HTTP API
2) Datenmodell + Migration/Schema
3) systemd service file + Config
4) Minimal-Doku / Runbook

---

## Roadmap / Schritte

### 0) Projekt-Setup
- Rust workspace oder single crate (empfohlen: single crate).
- `Cargo.toml` mit:
    - `actix-web`
    - `tokio` (für Timer + UDP; oder std + blocking UDP; tokio empfohlen)
    - `serde`, `serde_json`
    - SQLite: **`rusqlite`** (synchron) *oder* `sqlx` (async). Empfehlung: **rusqlite** (simpel, wenig Overhead).
    - `thiserror` (Fehler)
    - `tracing`, `tracing-subscriber` (Logging)
    - `config` oder `figment` (Konfiguration)
- Verzeichnisstruktur (Vorschlag):
    - `src/main.rs`
    - `src/config.rs`
    - `src/keba_udp.rs`
    - `src/poller.rs`
    - `src/db.rs`
    - `src/api.rs`
    - `src/models.rs`

### 1) Konfiguration
- Config via ENV + optional `config.toml`:
    - `KEBA_IP` (Wallbox IP)
    - `KEBA_UDP_PORT` (default 7090)
    - `POLL_INTERVAL_MS` (default 3000)
    - `DB_PATH` (z. B. `/var/lib/keba/keba.db`)
    - `HTTP_BIND` (z. B. `0.0.0.0:8080`)
    - Optional: `DEBOUNCE_SAMPLES` (default 2)
    - Optional: mögliche Key-Namen:
        - Status/Plug: `Plug`, `State` etc.
        - Seconds: `Seconds`, `Sec`, `seconds`
        - Session energy: `"E pres"` (Wh) oder `Energy (present session)` (kWh)
        - Total energy: `Total energy` (Wh) oder `Energy (total)` (kWh)

### 2) UDP Client (KEBA)
- Implementiere eine Funktion `send_cmd(cmd: &str) -> Result<serde_json::Value>`:
    - UDP send an `KEBA_IP:KEBA_UDP_PORT`
    - Receive mit Timeout (z. B. 2s)
    - Parse JSON, sonst Fehler/None
- Funktionen:
    - `get_report2()` (Status/Plug/Seconds)
    - `get_report3()` (Energy present session + optional totals)
- Parsing Helper:
    - Zugriff auf Keys mit Leerzeichen
    - tolerant: Number oder String
    - Optionale Fallbacks über mehrere Key-Kandidaten

### 3) State-Machine / Poller
- Poll loop:
    - Jede `POLL_INTERVAL_MS`: `report 2` holen
    - `plugged` bool ableiten:
        - primär aus `Plug` (Plug > 0)
        - fallback aus `State` (non-zero)
    - Debounce:
        - Status muss `DEBOUNCE_SAMPLES` mal gleich sein, bevor Transition akzeptiert wird
- Transition Handling:
    - `unplugged -> plugged`:
        - `plugged_at = now_utc()`
        - optional: `start_session_energy` merken (wenn present-session nicht bei 0 startet)
        - optional: `start_total_energy` merken (Alternative: total-diff)
    - `plugged -> unplugged`:
        - `unplugged_at = now_utc()`
        - `report 3` lesen:
            - `present_session_kwh` bestimmen:
                - falls `"E pres"` in Wh: /1000
                - falls direkt kWh: übernehmen
            - Wenn Startwert vorhanden: `kwh = max(0, end - start)`
            - else `kwh = end`
        - Session in DB schreiben
        - Memory state reset
- Reboot/Counter reset:
    - seconds counter kann fallen; ist aber nicht kritisch, da echte Zeiten vom Host kommen
    - optional: log warning bei seconds reset

### 4) SQLite Datenbank
- Schema (minimal):
    - Tabelle `sessions`:
        - `id INTEGER PRIMARY KEY AUTOINCREMENT`
        - `plugged_at TEXT NOT NULL` (ISO8601 UTC)
        - `unplugged_at TEXT NOT NULL` (ISO8601 UTC)
        - `kwh REAL NOT NULL`
        - `created_at TEXT NOT NULL` (ISO8601 UTC)
        - optional debug:
            - `raw_report2 TEXT` (JSON)
            - `raw_report3 TEXT` (JSON)
- Indizes:
    - `created_at` DESC
- DB Access Layer:
    - `insert_session(session) -> id`
    - `get_latest_session() -> Option<Session>`
    - `list_sessions(limit, offset)`

### 5) HTTP API (actix-web)
- Server startet parallel zum Poller (tokio tasks oder actix runtime).
- Endpoints:
    - `GET /health` → `{ "status": "ok" }`
    - `GET /sessions/latest` → letzte Session (oder 404)
    - `GET /sessions?limit=50&offset=0` → Liste
    - Optional: `GET /sessions/{id}`
- Response JSON:
  ```json
  {
    "id": 123,
    "pluggedAt": "2026-02-20T18:12:03.120Z",
    "unpluggedAt": "2026-02-20T22:45:10.002Z",
    "kwh": 10.83
  }
  ```

---

## Festgelegte Produktentscheidungen (2026-02-20)
1. Erst Roadmap/Plan finalisieren, dann Implementierung.
2. KEBA Parsing tolerant umsetzen (Alias-Keys + Unit-Heuristik).
3. Session-kWh primär aus `E pres`, `total-diff` als Fallback/Validierung.
4. `kwh` in DB als `NOT NULL`; bei Fallback nur explizit und mit Logging.
5. API v1 umfasst nur `GET /health`, `GET /sessions/latest`, `GET /sessions`.
6. v1 ohne Auth im Heimnetz, aber mit vorbereitetem Auth-Extension-Point.
7. Deployment v1: Raspberry Pi + systemd; Docker später optional.
8. Defaults übernehmen: `DB_PATH=/var/lib/keba/keba.db`, `HTTP_BIND=0.0.0.0:8080`.
9. Debounce-Default: `2` (konfigurierbar).
10. Kein Dev-Simulationsendpoint in v1-Prodpfad; optional später hinter Feature-Flag.
