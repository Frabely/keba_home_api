# RUNBOOK.md

## Zweck
Betriebsanleitung fuer `keba_home_api` auf Raspberry Pi/Linux mit `systemd`.

## Voraussetzungen
- Binary vorhanden unter `/opt/keba_home_api/keba_home_api`
- Service-User existiert: `keba`
- Persistenzpfad vorhanden und beschreibbar: `/var/lib/keba`
- systemd unit installiert: `deploy/systemd/keba-home-api.service`
- ENV-Datei erstellt aus `deploy/systemd/keba-home-api.env.example`

## Installation
1. Verzeichnisse vorbereiten:
```bash
sudo mkdir -p /opt/keba_home_api /etc/keba /var/lib/keba
sudo chown -R keba:keba /opt/keba_home_api /var/lib/keba
```
2. Binary deployen:
```bash
sudo install -m 0755 ./target/release/keba_home_api /opt/keba_home_api/keba_home_api
```
3. ENV-Datei anlegen:
```bash
sudo cp deploy/systemd/keba-home-api.env.example /etc/keba/keba-home-api.env
sudo chown root:root /etc/keba/keba-home-api.env
sudo chmod 0640 /etc/keba/keba-home-api.env
```
4. Service-Unit installieren:
```bash
sudo cp deploy/systemd/keba-home-api.service /etc/systemd/system/keba-home-api.service
sudo systemctl daemon-reload
sudo systemctl enable keba-home-api
```

## Start / Stop / Restart
```bash
sudo systemctl start keba-home-api
sudo systemctl stop keba-home-api
sudo systemctl restart keba-home-api
sudo systemctl status keba-home-api
```

## Logs (journald)
```bash
sudo journalctl -u keba-home-api -f
sudo journalctl -u keba-home-api --since "1 hour ago"
```

## Health-Check
```bash
curl -s http://127.0.0.1:8080/health
```
Erwartet: `{"status":"ok"}`

## Upgrade
1. Neue Version bauen/deployen:
```bash
cargo build --release
sudo install -m 0755 ./target/release/keba_home_api /opt/keba_home_api/keba_home_api
```
2. Optional ENV anpassen (`/etc/keba/keba-home-api.env`).
3. Neustart:
```bash
sudo systemctl restart keba-home-api
```
4. Verifikation:
```bash
sudo systemctl status keba-home-api
curl -s http://127.0.0.1:8080/health
```

## Backup / Restore (SQLite)
Hinweis: Fuer konsistente Backups Service vorher stoppen.

### Backup
```bash
sudo systemctl stop keba-home-api
sudo cp /var/lib/keba/keba.db /var/lib/keba/keba.db.bak-$(date +%Y%m%d-%H%M%S)
sudo systemctl start keba-home-api
```

### Restore
```bash
sudo systemctl stop keba-home-api
sudo cp /var/lib/keba/keba.db.bak-YYYYMMDD-HHMMSS /var/lib/keba/keba.db
sudo chown keba:keba /var/lib/keba/keba.db
sudo systemctl start keba-home-api
```

## Haeufige Fehlerbilder
1. Service startet nicht, `status=203/EXEC`:
- Binary-Pfad in `ExecStart` pruefen.
- Datei-Rechte pruefen (`chmod +x`).

2. DB-Fehler `unable to open database file`:
- Verzeichnis `/var/lib/keba` existiert und ist fuer User `keba` beschreibbar.
- `DB_PATH` in ENV-Datei pruefen.

3. Keine Daten trotz laufendem Service:
- `KEBA_IP`/Port pruefen.
- UDP-Erreichbarkeit im Heimnetz pruefen.
- Logs mit `journalctl -u keba-home-api -f` beobachten.

4. HTTP nicht erreichbar:
- `HTTP_BIND` pruefen (z. B. `0.0.0.0:8080`).
- Port-Freigabe/Firewall pruefen.

## Rollback-Hinweis
Bei Problemen auf vorheriges Binary zurueckwechseln und Service neu starten:
```bash
sudo install -m 0755 /opt/keba_home_api/keba_home_api.previous /opt/keba_home_api/keba_home_api
sudo systemctl restart keba-home-api
```
