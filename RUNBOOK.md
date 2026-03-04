# RUNBOOK.md

Hinweis: Die zentrale und kuenftig zu pflegende Setup-/Betriebsdoku liegt in [setup.md](C:\\Users\\MoritzDesktop\\IdeaProjects\\keba_home_api\\setup.md).

## Zweck
Betriebsanleitung fuer getrennten Betrieb von:
- `keba_service` (Writer, hier als zwei Instanzen fuer zwei Stationen)
- `keba_api` (Reader: liefert HTTP-API aus derselben SQLite)

## Architektur
- Gemeinsame DB: `DB_PATH=/var/lib/keba/keba.db`
- Writer-Instanzen schreiben Sessions/Log-Events
- API liest Sessions/Diagnostik aus derselben DB

## Voraussetzungen
- Binaries vorhanden unter `/opt/keba_home_api/`
: `keba_service`, `keba_api`
- Service-User: `keba`
- Persistenzpfad vorhanden und beschreibbar: `/var/lib/keba`
- systemd units installiert:
  - `deploy/systemd/keba-home-service@.service`
  - `deploy/systemd/keba-home-api-reader.service`
- ENV-Dateien erstellt aus:
  - `deploy/systemd/keba-home-service-carport.env.example`
  - `deploy/systemd/keba-home-service-eingang.env.example`
  - `deploy/systemd/keba-home-api-reader.env.example`

## Installation
1. Verzeichnisse vorbereiten:
```bash
sudo mkdir -p /opt/keba_home_api /etc/keba /var/lib/keba
sudo chown -R keba:keba /opt/keba_home_api /var/lib/keba
```

2. Binaries deployen:
```bash
cargo build --release -p keba-service -p keba-api
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

## Start / Stop / Restart
```bash
sudo systemctl start keba-home-service@carport
sudo systemctl start keba-home-service@eingang
sudo systemctl start keba-home-api-reader

sudo systemctl stop keba-home-api-reader
sudo systemctl stop keba-home-service@carport
sudo systemctl stop keba-home-service@eingang

sudo systemctl restart keba-home-service@carport
sudo systemctl restart keba-home-service@eingang
sudo systemctl restart keba-home-api-reader

sudo systemctl status keba-home-service@carport
sudo systemctl status keba-home-service@eingang
sudo systemctl status keba-home-api-reader
```

## Logs (journald)
```bash
sudo journalctl -u keba-home-service@carport -f
sudo journalctl -u keba-home-service@eingang -f
sudo journalctl -u keba-home-api-reader -f
```

## Health-Check
```bash
curl -s http://127.0.0.1:8080/health
curl -s http://127.0.0.1:8080/sessions/carport/latest
```

## Upgrade
1. Neue Version bauen/deployen:
```bash
cargo build --release -p keba-service -p keba-api
sudo install -m 0755 ./target/release/keba_service /opt/keba_home_api/keba_service
sudo install -m 0755 ./target/release/keba_api /opt/keba_home_api/keba_api
```

2. Dienste neu starten:
```bash
sudo systemctl restart keba-home-service@carport
sudo systemctl restart keba-home-service@eingang
sudo systemctl restart keba-home-api-reader
```

3. Verifikation:
```bash
sudo systemctl status keba-home-service@carport keba-home-service@eingang keba-home-api-reader
curl -s http://127.0.0.1:8080/health
```

## Backup / Restore (SQLite)
Hinweis: fuer konsistente Backups beide Dienste vorher stoppen.

### Backup
```bash
sudo systemctl stop keba-home-api-reader
sudo systemctl stop keba-home-service@carport
sudo systemctl stop keba-home-service@eingang
sudo cp /var/lib/keba/keba.db /var/lib/keba/keba.db.bak-$(date +%Y%m%d-%H%M%S)
sudo systemctl start keba-home-service@carport
sudo systemctl start keba-home-service@eingang
sudo systemctl start keba-home-api-reader
```

### Restore
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

## Haeufige Fehlerbilder
1. API startet, aber liefert DB-Fehler:
- Pruefen, ob mindestens eine Writer-Instanz bereits lief und Migrationen geschrieben hat.
- Writer zuerst starten, danach API.

2. Writer kann DB nicht oeffnen (`unable to open database file`):
- Rechte auf `/var/lib/keba` pruefen.
- `DB_PATH` in beiden ENV-Dateien pruefen.

3. API zeigt keine neuen Sessions:
- Writer-Logs pruefen: `journalctl -u keba-home-service@carport -f` und `...@eingang -f`
- KEBA-Erreichbarkeit/IP/Port pruefen.

## Rollback-Hinweis
Bei Problemen auf vorherige Binaries wechseln und beide Dienste neu starten:
```bash
sudo install -m 0755 /opt/keba_home_api/keba_service.previous /opt/keba_home_api/keba_service
sudo install -m 0755 /opt/keba_home_api/keba_api.previous /opt/keba_home_api/keba_api
sudo systemctl restart keba-home-service@carport
sudo systemctl restart keba-home-service@eingang
sudo systemctl restart keba-home-api-reader
```
