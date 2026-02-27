# KEBA Wallboxen (SMARTFOX-Setup) – Status & kWh per UDP auslesen

## Geräte / Adressen (LAN)
| Wallbox | IP (LAN) | UDP Port (KEBA UDP Interface) |
|---|---:|---:|
| KEBA (Carport) | `192.168.233.98` | `7090/UDP` |
| KEBA (Eingang) | `192.168.233.91` | `7090/UDP` |

> Für die Auswertung im Heimnetz nutzt du immer die **LAN-IP + UDP 7090**.

---

## Welche Reports liefern was?
- **`report 2`** → Zustände/Flags (Stecker, Freigaben, Limits, Fehler)
- **`report 3`** → Messwerte inkl. **Energie**:
    - `E pres` = Energie der aktuellen/letzten Session
    - `E total` = Gesamtzähler (persistent)

### kWh-Umrechnung (KEBA UDP typisch)
Die Werte sind i.d.R. in **0,1 Wh**:
- `kWh = Wert / 10000`

Beispiel von `192.168.233.98`:
- `E pres = 41210` → `41210 / 10000 = 4.121 kWh`
- `E total = 283467494` → `283467494 / 10000 = 28346.749 kWh`

---

## Statuslogik (praktisch)
Aus `report 2` kannst du stabil ableiten:

- **Plugged (angesteckt)**: `Plug != 0`
- **Enabled (freigegeben)**: `Enable sys == 1` UND `Enable user == 1` UND `Max curr > 0`
- **Fault (Fehler)**: `Error1 != 0` ODER `Error2 != 0`
- **Charging (lädt)**: am zuverlässigsten über **Leistung** (aus `report 3`: `P > 0`)  
  (State-Werte können je nach Firmware/Setup unterschiedlich interpretiert werden)

### Einordnung deiner aktuellen Werte
- `192.168.233.98`
    - `report 2`: `State=2`, `Plug=7`, `Enable sys=1`, `Enable user=1`, `Max curr=32000`
    - ⇒ **angesteckt**, **freigegeben**, **lädt aktuell nicht** (siehe `report 3`: `P=0`)
    - `report 3`: `P=0`, `E pres=41210`, `E total=283467494`

- `192.168.233.91`
    - `report 2`: `State=5`, `Plug=7`, `Enable sys=0`, `Enable user=0`, `Max curr=0`
    - ⇒ **angesteckt**, aber **gesperrt/deaktiviert (0 A freigegeben)**, kein Fehler (`Error1/2=0`)

---

## PowerShell: UDP Call (KEBA) – `report 2` / `report 3`
Wichtig: Viele Setups erwarten, dass dein PC **lokal auf UDP 7090** lauscht, damit die Antwort ankommt.

```powershell
function Get-KebaUdpReport {
  param(
    [Parameter(Mandatory=$true)][string]$Ip,
    [Parameter(Mandatory=$true)][int]$ReportId,
    [int]$TimeoutMs = 2000
  )

  $udp = New-Object System.Net.Sockets.UdpClient(7090)
  $udp.Client.ReceiveTimeout = $TimeoutMs

  try {
    $cmd = "report $ReportId"
    $bytes = [Text.Encoding]::ASCII.GetBytes($cmd)
    [void]$udp.Send($bytes, $bytes.Length, $Ip, 7090)

    $remote = New-Object System.Net.IPEndPoint([System.Net.IPAddress]::Any, 0)
    $respBytes = $udp.Receive([ref]$remote)
    [Text.Encoding]::ASCII.GetString($respBytes)
  }
  finally { $udp.Close() }
}

# Beispiele:
Get-KebaUdpReport -Ip "192.168.233.98" -ReportId 2
Get-KebaUdpReport -Ip "192.168.233.98" -ReportId 3
Get-KebaUdpReport -Ip "192.168.233.91" -ReportId 2
Get-KebaUdpReport -Ip "192.168.233.91" -ReportId 3