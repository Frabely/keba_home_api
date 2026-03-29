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
Wenn `$HOME/.cargo` oder `$HOME/.rustup` nicht schreibbar ist, nutzt das Skript automatisch `/tmp` als Fallback.
Falls `cargo` als rustup-shim ohne Default-Toolchain vorhanden ist, setzt das Skript automatisch `rustup default stable`.
Wenn der Systemuser `keba` fehlt, erstellt `setup_all.sh` ihn automatisch (abschaltbar mit `--no-create-user`).

2. Nach Setzen der beiden `KEBA_IP` Werte alle Prozesse starten:
```bash
bash ./scripts/start_all_services.sh
```

3. Vollstaendig aufraeumen (inkl. Build-Artefakte):
```bash
bash ./scripts/cleanup_all.sh
```
Hinweis: Entfernt Deploy-Artefakte, systemd-Units, ENV-Dateien und standardmaessig auch `/var/lib/keba` sowie `/var/backups/keba`.
Wenn Daten erhalten bleiben sollen:
```bash
bash ./scripts/cleanup_all.sh --keep-data
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

## Troubleshooting (Rust/Cargo Installation)
Bei Fehlern wie `Input/output error (os error 5)` liegt meist ein Speicher-/Dateisystemproblem vor.
Schnellcheck:
```bash
df -h
dmesg | tail -n 50
```
Falls das Dateisystem fehlerhaft ist: fsck beim naechsten Boot einplanen.

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
- `POLL_INTERVAL_MS` Default fuer Writer ist `20000` (20 Sekunden), falls der Wert in der ENV nicht gesetzt ist.
- Fuer die API-Endpunkte `GET /api/v1/sessions/carport/latest` und `GET /api/v1/sessions/entrance/latest` muss in der API-ENV `STATUS_STATIONS` beide Stationen mit Namen enthalten, die `carport` bzw. `entrance`/`eingang` matchen (z. B. `Carport@192.168.1.20:7090;Eingang@192.168.1.21:7090`).
- Die kanonischen API-Endpunkte liegen unter `/api/v1` (Legacy-Rootpfade bleiben zunaechst aktiv).
- Die API ist aktuell bewusst ohne Auth fuer Readonly-Zugriffe erreichbar.
- `CORS_ALLOWED_ORIGINS` steuert Browserzugriffe. Default sind `http://localhost:3000` und `https://invessiv.de`; fuer zusaetzliche produktive Frontends eine kommagetrennte Allow-List setzen.
- `DACHS_BASE_URL` steuert den HTTP-Upstream fuer `GET /api/v1/dachs/status`. Default ist `http://192.168.233.99:8080`; mit leerem Wert wird der Endpoint bewusst deaktiviert.
- `DACHS_USERNAME` und `DACHS_PASSWORD` sind optional. Wenn gesetzt, bettet die API sie direkt in die Dachs-Upstream-URL ein, also `http://user:pass@host/getKey?...`.
- Zum Schreiben der API-ENV auf dem Raspberry Pi liegt ein Hilfsskript unter `scripts/write_pi_api_env.sh`. Es schreibt standardmaessig nach `/etc/keba/keba-home-api-reader.env`; Werte koennen vor dem Aufruf per ENV ueberschrieben werden, z. B. `STATUS_STATIONS=... DACHS_USERNAME=... DACHS_PASSWORD=... bash scripts/write_pi_api_env.sh`.

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

Alle drei Services gemeinsam neu starten:
```bash
bash scripts/restart_services.sh
```
Das Script restartet alle drei Units, wartet auf `active` und zeigt danach Status + letzte Journal-Logs pro Service.

Deploy + Restart + Checks am Pi in einem Lauf:
```bash
chmod +x scripts/post_deploy_check.sh
bash scripts/post_deploy_check.sh
```
Wichtig:
- `post_deploy_check.sh` muss aus dem Git-Checkout ausgefuehrt werden (z. B. `~/repos/keba_home_api`), nicht aus `/opt/keba_home_api`.
- Das Script baut im Repo, installiert die Binaries in die von systemd verwendeten `ExecStart`-Pfade und startet danach alle Services neu.
- Die API-Pruefung validiert `/api/v1/health` strikt (`status=ok`) und testet den Session-Endpunkt ohne Auth-Header.

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
- Unplug-Logfelder `Started`/`Ended` werden minutengenau (`YYYY-MM-DD HH:MM`) formatiert.
- Unplug-Logfeld `Wh` wird mit genau einer Nachkommastelle gespeichert (`x.y`).
- `LOG_FORMAT` steuert die Ausgabeform:
  - `compact` (Default, empfohlen fuer Konsole/Journal)
  - `pretty` (mehrzeilig, human-readable)
  - `full` (inkl. target/module fuer tiefes Debugging)

Kompakte Loganzeige in `journalctl`:
```bash
sudo journalctl -u keba-home-service@carport -f -o cat
```

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
curl -s http://127.0.0.1:65109/api/v1/health
curl -s http://127.0.0.1:65109/api/v1/sessions/carport/latest
```

## Von ueberall erreichbar ohne Domain (direkt ueber oeffentliche IP)
Das ist der einfachste externe Betrieb, solange noch keine Domain vorhanden ist.

1. API auf allen Interfaces lassen:
```bash
sudo sed -i 's/^HTTP_BIND=.*/HTTP_BIND=0.0.0.0:65109/' /etc/keba/keba-home-api-reader.env
```

2. Fuer Browser-Zugriffe vom lokalen Dev-Frontend und optional weiteren bekannten Frontends die Allow-List setzen:
```bash
sudo sed -i 's|^CORS_ALLOWED_ORIGINS=.*|CORS_ALLOWED_ORIGINS=http://localhost:3000,https://invessiv.de|' /etc/keba/keba-home-api-reader.env
sudo systemctl restart keba-home-api-reader
```

3. Im Router TCP-Port `65109` auf die lokale Pi-IP weiterleiten, z. B.:
- extern `65109` -> Raspberry Pi `192.168.178.50:65109`

4. Extern erreichbare URLs:
```text
http://<OEFFENTLICHE_IP>:65109/api/v1/health
http://<OEFFENTLICHE_IP>:65109/api/v1/sessions/carport/latest
http://<OEFFENTLICHE_IP>:65109/api/v1/sessions/entrance/latest
http://<OEFFENTLICHE_IP>:65109/api/v1/unplug-log
```

Beispiel:
```text
http://84.123.45.67:65109/api/v1/health
```

5. Lokal und extern testen:
```bash
curl -i http://127.0.0.1:65109/api/v1/health
curl -i http://<OEFFENTLICHE_IP>:65109/api/v1/sessions/carport/latest
```

Hinweise:
- Ohne Domain gibt es in diesem Setup kein automatisches HTTPS/TLS.
- Wenn sich deine oeffentliche IP aendert, aendert sich auch die URL.
- Manche Anschluesse nutzen CGNAT oder blockieren eingehende Ports; dann ist direkter Internetzugriff trotz Port-Forwarding nicht moeglich.
- Dieses Setup laeuft unverschluesselt ueber HTTP und derzeit ohne Authentifizierung.

## Von ueberall erreichbar (empfohlen: Caddy + HTTPS)
Wenn die API wirklich aus dem Internet erreichbar sein soll, nicht den nackten Port `65109` direkt weiterleiten. Besser: API lokal halten und per Reverse Proxy auf `443` publizieren.

1. API intern auf Loopback binden:
```bash
sudo sed -i 's/^HTTP_BIND=.*/HTTP_BIND=127.0.0.1:65109/' /etc/keba/keba-home-api-reader.env
sudo systemctl restart keba-home-api-reader
```

2. CORS fuer dein lokales Frontend plus spaeteren Frontend-Ursprung setzen:
```bash
sudo sed -i 's|^CORS_ALLOWED_ORIGINS=.*|CORS_ALLOWED_ORIGINS=http://localhost:3000,https://invessiv.de,https://app.example.com|' /etc/keba/keba-home-api-reader.env
sudo systemctl restart keba-home-api-reader
```

3. Caddy auf dem Pi installieren und Beispielkonfiguration verwenden:
```bash
sudo mkdir -p /etc/caddy
sudo cp deploy/caddy/keba-home-api.Caddyfile.example /etc/caddy/Caddyfile
sudo nano /etc/caddy/Caddyfile
```
Danach Domain von `api.example.com` auf deine echte Domain/DynDNS aendern.

4. Router/Netz:
- DynDNS oder feste Domain auf deine oeffentliche IP zeigen lassen.
- TCP `80` und `443` auf den Raspberry Pi weiterleiten.
- Optional Pi-Firewall nur fuer `80/443` oeffnen; `65109` extern geschlossen lassen.

5. Caddy starten:
```bash
sudo systemctl enable --now caddy
sudo systemctl status caddy
```

6. Extern testen:
```bash
curl -i https://api.example.com/api/v1/health
curl -i https://api.example.com/api/v1/sessions/carport/latest
```

Hinweise:
- Damit ist die API per HTTPS von ueberall erreichbar, ohne den internen Actix-Port offenzulegen.
- HTTPS ist fuer den Internetbetrieb weiterhin der saubere Zielzustand, auch wenn Auth spaeter nachgezogen wird.

Rollback:
```bash
sudo sed -i 's/^HTTP_BIND=.*/HTTP_BIND=0.0.0.0:65109/' /etc/keba/keba-home-api-reader.env
sudo sed -i 's|^CORS_ALLOWED_ORIGINS=.*|CORS_ALLOWED_ORIGINS=http://localhost:3000,https://invessiv.de|' /etc/keba/keba-home-api-reader.env
sudo systemctl restart keba-home-api-reader
```

## SQLite Parallelzugriff
- Writer nutzen SQLite mit `WAL`, `busy_timeout`, `foreign_keys`.
- Die API liest die DB aktuell nicht mehr; Persistenz wird nur fuer `unplug_log_events` durch die Writer genutzt.
- Zwei Writer serialisieren ihre Writes.

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
- Auf `poller tick failed` Warnungen achten (zeigen Fetch/Parse-Probleme von `report 2` direkt im Log). Wiederholte Fehler werden gebuendelt, damit Zustandswechsel-Logs sichtbar bleiben.
- `unplug_log_events`-Insert passiert beim debouncten Uebergang `vehicleConnected true -> false` (Abstecken).
- Persistenzschema fuer `unplug_log_events`: `Id`, `Timestamp`, `Station`, `Started`, `Ended`, `Wh`, `CardId`.
- Fuer UDP-`Plug`-Werte gilt: `>=5` bedeutet Fahrzeug verbunden (`5/7`), `0/1/3` gilt als nicht fahrzeugverbunden. Damit wird bei festen Kabeln ein Wechsel `7 -> 3` korrekt als Abstecken erkannt.
- Der Wechsel wird erst nach `DEBOUNCE_SAMPLES` gleichen Polls (Default `3`) bestaetigt; ohne bestaetigten Zustandswechsel wird kein neuer Unplug-Eintrag angelegt.
- Beim Service-Start wird der initiale stabile Plug-Status direkt uebernommen. Startet der Service bei bereits verbundenem Fahrzeug, bleibt der interne Zustand sofort `true` und ein spaeteres Abziehen kann ohne vorheriges Zwischen-Event korrekt persistiert werden.
- Beim Abziehen werden Report `100..130` fuer Sessiondetails durchsucht. Falls unmittelbar nach dem Unplug noch kein vollstaendiger Report vorliegt, werden die 1xx-Reports mehrfach mit kurzem Abstand erneut gelesen (Retry-Fenster), bevor auf `n/a` gefallen wird.

4. Session mit `startedAt: null`:
- Erwartetes Verhalten, wenn der Service waehrend einer bereits laufenden/angesteckten Session gestartet wurde.
- In diesem Fall ist der exakte Session-Startzeitpunkt unbekannt; `finishedAt` und `kwh` werden weiterhin normal persistiert.
