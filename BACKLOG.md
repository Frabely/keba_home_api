# BACKLOG.md

## Ziel
Umsetzbares Step-by-Step-Backlog für den KEBA P30 Home Service mit Fokus auf Cleanup-first, Stabilität und testbare Architektur.

## Arbeitsmodus
1. Kleine, klar getrennte Changes pro Thema.
2. Erst Cleanup/Grundlagen, dann Features.
3. Jede Story mit Akzeptanzkriterien und Testfokus.

## Sprint 0: Cleanup und Fundament (Priorität: sehr hoch)

### 0.1 Projektstruktur aufräumen ✅
- Scope:
  - Modulstruktur gemäß Architekturzielen vorbereiten (`domain`, `adapters`, `app`).
  - `main.rs` auf orchestrierende Startlogik reduzieren.
- Done:
  - Struktur steht, Projekt baut weiterhin.
- Tests:
  - `cargo build`, `cargo test` (Smoke).

### 0.2 Fehler- und Logging-Baseline ✅
- Scope:
  - Einheitliche Fehler-Typen (`thiserror`) und `Result`-Flüsse.
  - `tracing` + strukturierte Startup-/Fehlerlogs.
- Done:
  - Keine stillen Fehlerpfade.
- Tests:
  - Unit-Tests für zentrale Fehlermappings.

### 0.3 Konfigurations-Baseline ✅
- Scope:
  - Zentrale Config (ENV + optional Datei), validierte Defaults.
  - Defaults gemäß Entscheidungen setzen.
- Done:
  - Service startet mit Defaults und validiert Pflichtfelder (`KEBA_IP`).
- Tests:
  - Unit-Tests für Config-Laden und Validierung.

## Sprint 1: Domain und Parsing (Priorität: hoch)

### 1.1 KEBA Payload Parsing robust machen ✅
- Scope:
  - Tolerante Key-Auflösung (Alias-Keys, Felder mit Leerzeichen).
  - Unit-Heuristik für Energie (Wh/kWh).
- Done:
  - Parser liefert normierte Domainwerte oder klaren Fehler.
- Tests:
  - Table-driven Unit-Tests für Varianten und malformed payloads.

### 1.2 Session-State-Machine implementieren ✅
- Scope:
  - Debounce-Logik, Transitionen `unplugged -> plugged -> unplugged`.
  - Zeitquelle abstrahieren (Clock-Interface).
- Done:
  - Deterministische Zustandsübergänge.
- Tests:
  - Unit-Tests für Debounce, Flattern, Reconnect, Counter-Reset.

### 1.3 kWh-Strategie final implementieren ✅
- Scope:
  - Primär `E pres`, Fallback `total-diff`, inklusive Plausibilitätschecks.
- Done:
  - Klar priorisierte Berechnungsstrategie mit Logging bei Fallback.
- Tests:
  - Unit-Tests für Normalfall, negative Diffs, fehlende Felder.

## Sprint 2: Persistenz und API v1 (Priorität: hoch)

### 2.1 SQLite Schema + Migrationen ✅
- Scope:
  - Tabelle `sessions` mit `kwh REAL NOT NULL`, Index auf `created_at`.
  - Migrationseinführung statt ad-hoc SQL.
- Done:
  - Frische DB und bestehende DB migrationsfähig.
- Tests:
  - Integrationstests gegen temporäre SQLite.

### 2.2 Repository-Layer ✅
- Scope:
  - `insert_session`, `get_latest_session`, `list_sessions(limit, offset)`.
- Done:
  - Stabile DB-API ohne Leaky-Details.
- Tests:
  - Integrationstests inkl. Pagination und leerer DB.

### 2.3 HTTP API v1 ✅
- Scope:
  - `GET /health`, `GET /sessions/latest`, `GET /sessions`.
  - Einheitliche Fehlerantworten und stabile JSON-Contracts.
- Done:
  - Endpoints liefern erwartete Statuscodes/Body.
- Tests:
  - HTTP-Integrationstests (Contract-Tests).

## Sprint 3: Runtime und Betrieb (Priorität: mittel)

### 3.1 Poller + API orchestrieren ✅
- Scope:
  - Sauberer Start/Shutdown, Task-Orchestrierung, Retry-Verhalten.
- Done:
  - Stabiler Dauerbetrieb ohne Deadlocks.
- Tests:
  - Integrationstest mit simuliertem UDP-Responder.

### 3.2 systemd + Runbook ✅
- Scope:
  - service unit, ENV-Datei-Beispiel, Betriebsdoku (Start/Restart/Logs/Backup).
- Done:
  - Deploy auf Raspberry Pi nachvollziehbar.
- Tests:
  - Manuelle Verifikation nach Checkliste.

### 3.3 CI-Automation ✅
- Scope:
  - CI mit `fmt`, `clippy -D warnings`, `test`.
- Done:
  - Pipeline grün als Merge-Voraussetzung.
- Tests:
  - CI-Lauf erfolgreich.

## Reihenfolge der Umsetzung (Step-by-Step)
1. 0.1 Projektstruktur aufräumen
2. 0.2 Fehler- und Logging-Baseline
3. 0.3 Konfigurations-Baseline
4. 1.1 KEBA Parsing robust machen
5. 1.2 Session-State-Machine implementieren
6. 1.3 kWh-Strategie final implementieren
7. 2.1 SQLite Schema + Migrationen
8. 2.2 Repository-Layer
9. 2.3 HTTP API v1
10. 3.1 Poller + API orchestrieren
11. 3.2 systemd + Runbook
12. 3.3 CI-Automation

## Risiken und frühe Gegenmaßnahmen
1. Feldvarianten am Gerät: früh mit Sample-Payloads absichern, Parser tolerant halten.
2. Flatternde Zustände: Debounce + Regressionstests von Anfang an.
3. Datenintegrität: Migrationen + `NOT NULL` + explizite Fallback-Logs.
4. Betriebsstabilität: klare Retry-/Timeout-Strategie, Health endpoint und strukturierte Logs.
