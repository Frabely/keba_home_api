param(
    [Parameter(Mandatory = $true)]
    [string]$Ip,
    [int]$Port = 502,
    [int]$UnitId = 255,
    [int]$TimeoutMs = 2000,
    [int]$Attempts = 2,
    [double]$EnergyFactorWh = 0.1
)

$ErrorActionPreference = 'Stop'

function Read-InputU32 {
    param(
        [Parameter(Mandatory = $true)] [string]$Ip,
        [Parameter(Mandatory = $true)] [int]$Port,
        [Parameter(Mandatory = $true)] [byte]$UnitId,
        [Parameter(Mandatory = $true)] [int]$TimeoutMs,
        [Parameter(Mandatory = $true)] [uint16]$TransactionId,
        [Parameter(Mandatory = $true)] [uint16]$Address
    )

    $client = [System.Net.Sockets.TcpClient]::new()
    try {
        $iar = $client.BeginConnect($Ip, $Port, $null, $null)
        if (-not $iar.AsyncWaitHandle.WaitOne($TimeoutMs)) {
            throw "TCP connect timeout (${TimeoutMs}ms)"
        }
        $client.EndConnect($iar)
        $client.ReceiveTimeout = $TimeoutMs
        $client.SendTimeout = $TimeoutMs

        $stream = $client.GetStream()

        # MBAP(7) + PDU(5): function 0x04 (Read Input Registers), quantity=2
        $request = [byte[]]::new(12)
        $request[0] = [byte](($TransactionId -shr 8) -band 0xFF)
        $request[1] = [byte]($TransactionId -band 0xFF)
        $request[2] = 0x00
        $request[3] = 0x00
        $request[4] = 0x00
        $request[5] = 0x06
        $request[6] = [byte]$UnitId
        $request[7] = 0x04
        $request[8] = [byte](($Address -shr 8) -band 0xFF)
        $request[9] = [byte]($Address -band 0xFF)
        $request[10] = 0x00
        $request[11] = 0x02

        $stream.Write($request, 0, $request.Length)

        $header = [byte[]]::new(7)
        $read = $stream.Read($header, 0, 7)
        if ($read -ne 7) {
            throw "Unexpected MBAP header length: $read"
        }

        $len = ($header[4] -shl 8) -bor $header[5]
        if ($len -lt 3) {
            throw "Invalid Modbus length field: $len"
        }

        $pdu = [byte[]]::new($len - 1)
        $offset = 0
        while ($offset -lt $pdu.Length) {
            $chunk = $stream.Read($pdu, $offset, $pdu.Length - $offset)
            if ($chunk -le 0) {
                throw "Unexpected end of stream"
            }
            $offset += $chunk
        }

        $function = $pdu[0]
        if ($function -eq 0x84) {
            $exceptionCode = $pdu[1]
            throw "Modbus exception 0x{0:X2}" -f $exceptionCode
        }
        if ($function -ne 0x04) {
            throw "Unexpected function code: 0x{0:X2}" -f $function
        }
        if ($pdu[1] -ne 4) {
            throw "Unexpected byte count: $($pdu[1])"
        }

        $value =
            (($pdu[2] -shl 24) -bor
             ($pdu[3] -shl 16) -bor
             ($pdu[4] -shl 8) -bor
             $pdu[5])

        return [uint32]$value
    }
    finally {
        if ($client.Connected) {
            $client.Close()
        }
    }
}

Write-Host "KEBA Modbus diagnostics"
Write-Host "Target: $Ip`:$Port, UnitId: $UnitId, Timeout: ${TimeoutMs}ms, Attempts: $Attempts"
Write-Host ""

$registers = @(
    @{ Name = 'State'; Address = 1000; Kind = 'state' },
    @{ Name = 'TotalEnergyRaw'; Address = 1036; Kind = 'energy' },
    @{ Name = 'PresentSessionRaw'; Address = 1502; Kind = 'energy' }
)

for ($attempt = 1; $attempt -le $Attempts; $attempt++) {
    Write-Host "Attempt $attempt/$Attempts"
    foreach ($reg in $registers) {
        try {
            $tx = [uint16](($attempt * 10) + $reg.Address % 10)
            $raw = Read-InputU32 -Ip $Ip -Port $Port -UnitId ([byte]$UnitId) -TimeoutMs $TimeoutMs -TransactionId $tx -Address ([uint16]$reg.Address)

            if ($reg.Kind -eq 'energy') {
                $kwh = ($raw * $EnergyFactorWh) / 1000.0
                Write-Host ("  {0} (reg {1}): raw={2}, approx={3:N3} kWh" -f $reg.Name, $reg.Address, $raw, $kwh)
            }
            else {
                Write-Host ("  {0} (reg {1}): {2}" -f $reg.Name, $reg.Address, $raw)
            }
        }
        catch {
            Write-Host ("  {0} (reg {1}): ERROR -> {2}" -f $reg.Name, $reg.Address, $_.Exception.Message)
        }
    }
    Write-Host ""
}

Write-Host "Interpretation:"
Write-Host "- If all registers fail: Modbus TCP likely disabled, wrong UnitId, wrong port, or firewall/network block."
Write-Host "- If State works but energy fails: register mapping may differ by firmware."
Write-Host "- If all work: set KEBA_SOURCE=modbus and start the app."
