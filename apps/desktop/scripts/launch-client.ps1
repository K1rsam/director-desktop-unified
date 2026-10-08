param([string]$ReleaseDirectory)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
if (-not $ReleaseDirectory) { $ReleaseDirectory = Join-Path $projectRoot '.local\client' }
$clientPath = Join-Path ([System.IO.Path]::GetFullPath($ReleaseDirectory)) 'director-desktop-next.exe'
if (-not (Test-Path -LiteralPath $clientPath -PathType Leaf)) { throw '尚无独立客户端，请先构建、验收并发布 director-desktop-next.exe。' }
Start-Process -FilePath $clientPath -WorkingDirectory $projectRoot
