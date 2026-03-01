# setup.md

## Zweck
Zentrale Setup- und Betriebsanleitung fuer dieses Projekt.

Ab jetzt gilt: Wichtige Betriebs-/Deploy-/Raspberry-Informationen zuerst hier pflegen.

## Zielarchitektur
Es laufen drei Prozesse:
1. `keba_service` Instanz `carport` (Writer)
2. `keba_service` Instanz `eingang` (Writer)
3. `keba_api` (Reader)

Alle drei nutzen dieselbe SQLite-DB (`DB_PATH` identisch).

## Quickstart (2 Befehle)
1. Setup/Installation:
```bash
bash ./scripts/setup_all.sh
```
Hinweis: Wenn `cargo` fehlt, installiert `setup_all.sh` die Rust-Toolchain automatisch via `rustup` (benoetigt `curl` + Internetzugang).

2. Nach Setzen der beiden `KEBA_IP` Werte alle Prozesse starten:
```bash
bash ./scripts/start_all_services.sh
```

## Projektstruktur (relevant)
- `keba-service/` eigene ausführbare Crate fuer Writer
- `keba-api/` eigene ausführbare Crate fuer API
- `src/` gemeinsame Kernlogik (Domain/App/Adapter)
- `deploy/systemd/` Units + ENV-Beispiele
- `scripts/setup_all.sh` einmaliger Setup
- `scripts/start_all_services.sh` Start von 2x Writer + API

## Build
```bash
cargo build --release -p keba-service -p keba-api
```

Artefakte:
- `target/release/keba_service`
- `target/release/keba_api`

## Linux/Raspberry Installation
Hinweis: Dieser Abschnitt ist der manuelle Fallback.
Bevorzugt `Quickstart (2 Befehle)` verwenden.

1. Verzeichnisse anlegen:
```bash
sudo mkdir -p /opt/keba_home_api /etc/keba /var/lib/keba
sudo chown -R keba:keba /opt/keba_home_api /var/lib/keba
```

2. Binaries deployen:
```bash
sudo install -m 0755 ./target/release/keba_service /opt/keba_home_api/keba_service
sudo install -m 0755 ./target/release/keba_api /opt/keba_home_api/keba_api
```

3. ENV-Dateien anlegen:
```bash
sudo cp deploy/systemd/keba-home-service-carport.env.example /etc/keba/keba-home-service-carport.env
sudo cp deploy/systemd/keba-home-service-eingang.env.example /etc/keba/keba-home-service-eingang.env
sudo cp deploy/systemd/keba-home-api-reader.env.example /etc/keba/keba-home-api-reader.env
sudo chown root:root /etc/keba/keba-home-service-carport.env /etc/keba/keba-home-service-eingang.env /etc/keba/keba-home-api-reader.env
sudo chmod 0640 /etc/keba/keba-home-service-carport.env /etc/keba/keba-home-service-eingang.env /etc/keba/keba-home-api-reader.env
```

Wichtig:
- `KEBA_IP` ist fuer jede Writer-Instanz verpflichtend.
- Setze `KEBA_IP` in `/etc/keba/keba-home-service-carport.env` auf die Carport-Wallbox-IP.
- Setze `KEBA_IP` in `/etc/keba/keba-home-service-eingang.env` auf die Eingang-Wallbox-IP.

4. Sicherstellen, dass alle dieselbe DB nutzen:
```bash
grep '^DB_PATH=' /etc/keba/keba-home-service-carport.env /etc/keba/keba-home-service-eingang.env /etc/keba/keba-home-api-reader.env
```

5. systemd Units installieren:
```bash
sudo cp deploy/systemd/keba-home-service@.service /etc/systemd/system/keba-home-service@.service
sudo cp deploy/systemd/keba-home-api-reader.service /etc/systemd/system/keba-home-api-reader.service
sudo systemctl daemon-reload
sudo systemctl enable keba-home-service@carport keba-home-service@eingang keba-home-api-reader
```

## Start / Stop / Status
Start:
```bash
sudo systemctl start keba-home-service@carport
sudo systemctl start keba-home-service@eingang
sudo systemctl start keba-home-api-reader
```

Stop:
```bash
sudo systemctl stop keba-home-api-reader
sudo systemctl stop keba-home-service@carport
sudo systemctl stop keba-home-service@eingang
```

Status:
```bash
sudo systemctl status keba-home-service@carport
sudo systemctl status keba-home-service@eingang
sudo systemctl status keba-home-api-reader
```

Logs:
```bash
sudo journalctl -u keba-home-service@carport -f
sudo journalctl -u keba-home-service@eingang -f
sudo journalctl -u keba-home-api-reader -f
```
Log-Verhalten (Default `RUST_LOG=info`):
- `INFO`: Startup + Zustandsaenderungen + Session-Lifecycle + Heartbeat (standardmaessig 1x/Minute via `STATUS_LOG_INTERVAL_SECONDS=60`)
- `WARN/ERROR`: sofort bei Problemen
- `DEBUG`: detaillierte Poll-/Request-Details nur bei aktivem Debug-Level

Debug temporär aktivieren (beide Writer + API), dann zurueck auf `info`:
```bash
sudo sed -i 's/^RUST_LOG=.*/RUST_LOG=debug/' /etc/keba/keba-home-service-carport.env
sudo sed -i 's/^RUST_LOG=.*/RUST_LOG=debug/' /etc/keba/keba-home-service-eingang.env
sudo sed -i 's/^RUST_LOG=.*/RUST_LOG=debug/' /etc/keba/keba-home-api-reader.env
sudo systemctl restart keba-home-service@carport keba-home-service@eingang keba-home-api-reader

# rollback auf normalen Betrieb
sudo sed -i 's/^RUST_LOG=.*/RUST_LOG=info/' /etc/keba/keba-home-service-carport.env
sudo sed -i 's/^RUST_LOG=.*/RUST_LOG=info/' /etc/keba/keba-home-service-eingang.env
sudo sed -i 's/^RUST_LOG=.*/RUST_LOG=info/' /etc/keba/keba-home-api-reader.env
sudo systemctl restart keba-home-service@carport keba-home-service@eingang keba-home-api-reader
```

## API Smoke Check
```bash
curl -s http://127.0.0.1:8080/health
curl -s http://127.0.0.1:8080/diagnostics/db
```

## SQLite Parallelzugriff
- Writer nutzen SQLite mit `WAL`, `busy_timeout`, `foreign_keys`.
- API oeffnet die DB read-only (`query_only=ON`).
- Viele Leser + ein Schreiber gleichzeitig sind moeglich.
- Zwei Writer serialisieren ihre Writes.
- Bei `BUSY/LOCKED` wird Session-Persistenz mit Backoff mehrfach retryt.

## Raspberry Pi Hinweise

### Dauerbetrieb (Pi 4)
- 2 Writer + 1 API sind normalerweise unkritisch fuer CPU/RAM.
- Kritischer ist meist Storage (microSD Verschleiss) und Thermik.

### Temperatur/Throttling pruefen
Einmalig:
```bash
vcgencmd measure_temp
vcgencmd get_throttled
```

Live-Ansicht:
```bash
watch -n 2 'vcgencmd measure_temp; vcgencmd get_throttled'
```

Richtwerte:
- dauerhaft < 70C: gut
- Richtung 80C+: beobachten/gegensteuern
- `get_throttled=0x0`: kein Throttling/UV-Problem

### Journald begrenzen (wichtig)
Damit Logs den Speicher nicht vollschreiben:
```bash
sudo nano /etc/systemd/journald.conf
```

Empfohlene Werte:
```ini
[Journal]
SystemMaxUse=500M
SystemKeepFree=1G
MaxFileSec=1month
```

Aktivieren:
```bash
sudo systemctl restart systemd-journald
```

Pruefen/Bereinigen:
```bash
journalctl --disk-usage
sudo journalctl --vacuum-size=500M
```

### microSD Empfehlung
- 64/65GB ist fuer Start ok.
- Fuer langfristig robusten 24/7 Betrieb besser SSD (USB) fuer DB/Logs.

## Automatisierte taegliche Backups (empfohlen)
1. Script deployen:
```bash
sudo mkdir -p /opt/keba_home_api/scripts /var/backups/keba
sudo install -m 0755 ./scripts/backup_keba_db.sh /opt/keba_home_api/scripts/backup_keba_db.sh
sudo chown -R keba:keba /var/backups/keba
```

2. Backup-ENV anlegen:
```bash
sudo cp deploy/systemd/keba-db-backup.env.example /etc/keba/keba-db-backup.env
sudo chown root:root /etc/keba/keba-db-backup.env
sudo chmod 0640 /etc/keba/keba-db-backup.env
```

3. systemd Backup-Unit/Timer installieren:
```bash
sudo cp deploy/systemd/keba-db-backup.service /etc/systemd/system/keba-db-backup.service
sudo cp deploy/systemd/keba-db-backup.timer /etc/systemd/system/keba-db-backup.timer
sudo systemctl daemon-reload
sudo systemctl enable --now keba-db-backup.timer
```

4. Verifikation:
```bash
systemctl list-timers keba-db-backup.timer
sudo systemctl start keba-db-backup.service
ls -lah /var/backups/keba
```

Hinweis:
- Es wird ein konsistenter Online-Backup mit `sqlite3 .backup` erstellt.
- Alte Backups werden gemaess `KEEP_DAYS` automatisch geloescht.
- Mit `KEEP_DAYS=7` und taeglichem Timer werden Backups ab dem 8. Tag automatisch aufgeraeumt.

## Backup / Restore
Backup:
```bash
sudo systemctl stop keba-home-api-reader
sudo systemctl stop keba-home-service@carport
sudo systemctl stop keba-home-service@eingang
sudo cp /var/lib/keba/keba.db /var/lib/keba/keba.db.bak-$(date +%Y%m%d-%H%M%S)
sudo systemctl start keba-home-service@carport
sudo systemctl start keba-home-service@eingang
sudo systemctl start keba-home-api-reader
```

Restore:
```bash
sudo systemctl stop keba-home-api-reader
sudo systemctl stop keba-home-service@carport
sudo systemctl stop keba-home-service@eingang
sudo cp /var/lib/keba/keba.db.bak-YYYYMMDD-HHMMSS /var/lib/keba/keba.db
sudo chown keba:keba /var/lib/keba/keba.db
sudo systemctl start keba-home-service@carport
sudo systemctl start keba-home-service@eingang
sudo systemctl start keba-home-api-reader
```

## Troubleshooting kurz
1. API startet, aber DB-Fehler:
- Mindestens eine Writer-Instanz zuerst starten (Migrationen).

2. `database is locked` taucht auf:
- Kurzzeitige Contentions sind normal.
- Bei haeufigen Locks Poll-Intervall erhoehen oder I/O-Last reduzieren.

3. Keine neuen Sessions in API:
- Writer-Logs der betroffenen Station pruefen.
- IP/Port/KEBA-Erreichbarkeit pruefen.
