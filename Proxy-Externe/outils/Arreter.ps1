$processus = Get-Process -Name apivault-proxy -ErrorAction SilentlyContinue

if (-not $processus) {
    "Deja arrete."
    exit 0
}

$processus | Stop-Process -Force
Start-Sleep -Milliseconds 600

if (Get-Process -Name apivault-proxy -ErrorAction SilentlyContinue) {
    "Arret incomplet : un processus repond encore."
} else {
    "Arrete."
}
