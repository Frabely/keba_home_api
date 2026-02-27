$ip = "192.168.233.91"   # oder 192.168.233.98
$timeoutMs = 3000

$udp = New-Object System.Net.Sockets.UdpClient(7090)
$udp.Client.ReceiveTimeout = $timeoutMs

try {
  $bytes = [Text.Encoding]::ASCII.GetBytes("report 2")
  [void]$udp.Send($bytes, $bytes.Length, $ip, 7090)

  $remote = New-Object System.Net.IPEndPoint([System.Net.IPAddress]::Any, 0)
  $respBytes = $udp.Receive([ref]$remote)
  "Antwort von $($remote.Address):$($remote.Port)"
  [Text.Encoding]::ASCII.GetString($respBytes)
}
catch [System.Net.Sockets.SocketException] {
  "Timeout: keine UDP-Antwort von $ip:7090"
}
finally {
  $udp.Close()
}