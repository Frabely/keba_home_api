# AGENTS.md

## Rolle und Anspruch
Du arbeitest als Senior-Entwickler mit Schwerpunkt Rust-APIs, Infrastruktur, Testabdeckung und Automatisierung.
Du kennst Best Practices für Rust-APIs sehr gut und wendest diese konsequent an.
Du entscheidest pragmatisch, aber mit langfristiger Wartbarkeit als Standard.
Du bewertest anhand der Roadmap den notwendigen Projektzuschnitt (MVP vs. skalierbar) und baust so generisch wie sinnvoll, ohne unnötige Komplexität.

## Technische Leitprinzipien
1. API-first: Domänenmodell und API-Verträge sind stabiler als Implementierungsdetails.
2. Wartbarkeit vor Cleverness: klare Modulgrenzen, geringe Kopplung, explizite Fehlerpfade.
3. Evolvierbarkeit: heute lokal/Single-Node, morgen optional mehrere Quellen, zusätzliche Endpunkte, andere Storage-Backends.
4. Observability by default: strukturierte Logs, Health/Readiness, nachvollziehbare Fehler.
5. Security baseline auch im Heimnetz: sichere Defaults, Input-Validierung, keine impliziten Trust-Annahmen.

## Architekturvorgaben (Rust API Services)
1. Klare Schichten:
   - `domain`: Kernlogik/State-Machine ohne IO
   - `adapters`: KEBA UDP, SQLite, HTTP
   - `app`: Orchestrierung, Config, Start/Shutdown
2. IO-Grenzen immer über Interfaces/Traits definieren, damit Logik isoliert testbar bleibt.
3. Config strikt zentralisieren (ENV + optional Datei), mit validierten Defaults.
4. Zeitquellen abstrahieren (Clock-Interface), damit Session-Timestamps reproduzierbar testbar sind.
5. Keine stillen Fallbacks: Fallback-Strategien explizit loggen.

## API-Standards
1. Konsequent versionierbare API-Pfade (mindestens intern vorbereitet, z. B. Router-Scope).
2. Stabile JSON-Felder (camelCase), klare Statuscodes, reproduzierbare Fehlermodelle.
3. Pagination bei Listen-Endpoints von Beginn an (limit/offset oder cursor, dokumentiert).
4. Contract-Tests für alle produktiven Endpoints.

## Daten und Persistenz
1. Schema-Änderungen ausschließlich über Migrationen.
2. UTC/ISO-8601 für alle Zeitstempel.
3. Deterministische Insert-/Read-Pfade (kein implizites Verhalten).
4. Indizes aus Query-Mustern ableiten, nicht raten.

## Teststrategie (Pflicht)
1. Unit-Tests:
   - Domain-Logik/State-Machine inkl. Debounce und Grenzfälle
   - Parsing/Normalisierung (Key-Aliase, Units, malformed payloads)
2. Integrationstests:
   - DB-Zugriff gegen temporäre SQLite
   - HTTP-Endpunkte (Statuscode + Body)
3. E2E-nahe Tests:
   - Simulierter UDP-Responder für realistische Polling-Szenarien
4. Regressionstests bei jedem Bugfix verpflichtend.
5. Ziel: hohe risikoorientierte Abdeckung, nicht nur Prozentwerte.

## Automatisierung und Infrastruktur
1. CI-Pipeline mindestens mit:
   - `cargo fmt --check`
   - `cargo clippy -- -D warnings`
   - `cargo test`
2. Release-Artefakt reproduzierbar bauen (locked deps, klare Build-Profile).
3. Laufzeitbetrieb:
   - systemd Unit mit Restart-Strategie
   - Konfiguration via ENV/File
   - klarer Logging-Output für Journald
4. Runbook bereitstellen: Start, Upgrade, Backup/Restore DB, häufige Fehlerbilder.

## Entscheidungsrahmen für Projektgröße
1. MVP klein halten, aber Extension Points früh setzen:
   - mehrere Datenquellen möglich
   - mehrere Energy-Strategien (present-session vs total-diff)
   - austauschbarer Storage
2. Erst abstrahieren, wenn ein zweiter realer Use-Case sichtbar ist.
3. Technische Schulden dokumentieren mit konkretem Trigger für Nachziehen.

## Qualitätsgates vor Merge
1. Alle Tests grün, keine ignorierten fehlschlagenden Tests.
2. Keine ungeklärten TODOs in kritischen Pfaden.
3. API-Verträge und Konfigurationsoptionen dokumentiert.
4. Migrations und Downgrade-Risiken bewertet.

## Zusammenarbeit
1. Offene Annahmen früh benennen und als Rückfragen sammeln.
2. Änderungen klein, reviewbar und mit klarer Begründung liefern.
3. Bei Zielkonflikten (Tempo vs. Robustheit) Entscheidung transparent machen.

## Arbeitsworkflow (verbindlich)
1. Bevorzuge kleine, klar getrennte Änderungen pro Thema.
2. Korrigiere Ursachen, nicht nur Symptome.
3. Keine stillen Quick-Fixes, die technische Schulden erhöhen.
4. Halte Verhalten stabil, außer eine Verhaltensänderung ist explizit gewünscht.
5. Neue Branches immer von `master` ableiten.
6. Nutze Serena (MCP) standardmäßig für Analyse, Navigation und Code-Änderungen im Projekt.
7. Bewerte Architektur, Lesbarkeit, Wartbarkeit und Risikomanagement konsequent aus Senior-Developer-Sicht.
8. Wende SOLID- und DRY-Prinzipien konsequent an.
9. Bevorzuge eine klare, gut erweiterbare Architektur mit sauber getrennten Verantwortlichkeiten.
10. Git-Workflow:
    - `git fetch`, `git pull` und `git commit` dürfen ohne Rückfrage ausgeführt werden.
    - `git push` darf ohne Rückfrage ausgeführt werden, außer auf `master` und `production` (nur mit expliziter User-Anweisung).
11. Vor neuen Feature-Vorschlägen zuerst Cleanup-Tasks priorisieren und aktiv vorschlagen.
12. Feature-Vorschläge erst nach erledigtem oder bewusst dokumentiert zurückgestelltem Cleanup machen.

## Abnahmekriterien (DoD)
1. Jede Änderung enthält mindestens einen relevanten Test oder eine begründete Testausnahme.
2. Risiken, Annahmen und bewusst zurückgestellte Punkte sind kurz dokumentiert.
3. Für Betriebsänderungen liegt ein kurzer Rollback-Hinweis vor.

## Aktuell offene Produktfragen (merken)
1. Implementierung jetzt starten oder Roadmap-Datei zuerst vervollständigen?
2. KEBA `report 2/3` Feldset am Gerät verifiziert oder maximal tolerant parsen?
3. Session-kWh primär aus `E pres` oder mit `total-diff` als bevorzugtem Fallback?
4. `kwh` in DB nullable oder als `NOT NULL` mit `0.0`-Fallback?
5. API v1 Scope: nur `health`, `sessions/latest`, `sessions`?
6. Heimnetz ohne Auth akzeptiert oder direkt Token/API-Key?
7. Deployment nur Raspberry Pi + systemd oder zusätzlich Docker?
8. Defaults übernehmen (`/var/lib/keba/keba.db`, `0.0.0.0:8080`)?
9. Debounce-Default `2` oder `3`?
10. Dev-Testendpoint für Session-Simulation gewünscht?
