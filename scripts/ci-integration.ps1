$ErrorActionPreference = "Stop"
$env:FRAME_WIPE_FORBID_REAL_DISKS = "1"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
$Bin = Join-Path $Root "target\release\framewipe.exe"
if (-not (Test-Path $Bin)) { $Bin = Join-Path $Root "target\debug\framewipe.exe" }
$Work = New-Item -ItemType Directory -Path (Join-Path $env:TEMP ("framewipe-ci-" + [guid]::NewGuid().ToString("n")))
$env:FRAME_WIPE_TEST_DIR = $Work.FullName
Write-Host "binary: $Bin"
Write-Host "workdir: $($Work.FullName)"

& $Bin list --json | Out-Null
& $Bin list --all --json | Out-Null

$Img = Join-Path $Work.FullName "disk.img"
$bytes = New-Object byte[] (32 * 1024 * 1024)
[System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
[System.IO.File]::WriteAllBytes($Img, $bytes)

& $Bin wipe $Img --yes --confirm disk.img --countdown 0 --method zeros --report (Join-Path $Work.FullName "report")
if ($LASTEXITCODE -ne 0) { throw "wipe zeros failed" }
$zero = [System.IO.File]::ReadAllBytes($Img)
if ($zero | Where-Object { $_ -ne 0 }) { throw "image was not zeroed" }
Write-Host "image verified all zeros"

& $Bin wipe $Img --yes --confirm disk.img --countdown 0 --method random
if ($LASTEXITCODE -ne 0) { throw "wipe random failed" }

$Doc = Join-Path $Work.FullName "doc.txt"
Set-Content -Path $Doc -Value "secret" -NoNewline
& $Bin files $Doc --yes --confirm doc.txt --countdown 0
if ($LASTEXITCODE -ne 0) { throw "file wipe failed" }

# Refusal of a physical drive path (restricted mode / system disk).
& $Bin wipe "\\.\PhysicalDrive0" --yes --confirm PhysicalDrive0 --countdown 0 --dry-run
if ($LASTEXITCODE -eq 0) { throw "unexpected success planning a wipe of PhysicalDrive0" }
Write-Host "PhysicalDrive0 correctly refused"

# Optional VHD free-space test (often works on GitHub-hosted Windows, which is elevated).
$vhd = Join-Path $Work.FullName "test.vhdx"
try {
  New-VHD -Path $vhd -SizeBytes 64MB -Fixed -ErrorAction Stop | Out-Null
  $disk = Mount-VHD -Path $vhd -Passthru | Get-Disk
  Initialize-Disk -Number $disk.Number -PartitionStyle MBR
  $part = New-Partition -DiskNumber $disk.Number -UseMaximumSize -AssignDriveLetter
  Format-Volume -Partition $part -FileSystem NTFS -Confirm:$false -NewFileSystemLabel FWTEST | Out-Null
  $letter = $part.DriveLetter + ":\"
  $name = $part.DriveLetter.ToString() + ":"
  & $Bin free-space $letter --yes --confirm $name --countdown 0
  Dismount-VHD -Path $vhd -ErrorAction SilentlyContinue
  Write-Host "VHD free-space path finished with exit $LASTEXITCODE"
} catch {
  Write-Host "skipping VHD free-space test: $_"
}

Remove-Item -Recurse -Force $Work.FullName
Write-Host "Windows integration OK"
