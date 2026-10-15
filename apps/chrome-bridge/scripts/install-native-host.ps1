param(
    [string]$HostBinary,
    [string]$ComponentDirectory = (Join-Path $env:LOCALAPPDATA 'Programs\Director-3008-ChatGPT-Bridge\671815e'),
    [string]$InstallRoot = (Join-Path $env:LOCALAPPDATA 'Programs\Director-Native-Browser-Host'),
    [string]$EdgeUserDataRoot = (Join-Path $env:LOCALAPPDATA 'Microsoft\Edge\User Data'),
    [string]$ChromeUserDataRoot = (Join-Path $env:LOCALAPPDATA 'Google\Chrome\User Data'),
    [switch]$PlanOnly
)
$ErrorActionPreference = 'Stop'
$hostName = 'app.director.browser_bridge'
function Get-NativeFileSha([string]$Path) {
    $hash = [Security.Cryptography.SHA256]::Create()
    $stream = [IO.File]::OpenRead($Path)
    try { return [BitConverter]::ToString($hash.ComputeHash($stream)).Replace('-', '').ToLowerInvariant() }
    finally { $stream.Dispose(); $hash.Dispose() }
}
function Read-NativeJson([string]$Path, [string]$Code) {
    try { return ConvertFrom-Json -InputObject ([IO.File]::ReadAllText($Path, [Text.Encoding]::UTF8)) -ErrorAction Stop }
    catch { throw $Code }
}
if (-not $HostBinary) {
    $HostBinary = Join-Path $PSScriptRoot '..\..\native-browser-host\target\release\director-native-browser-host.exe'
}
function Assert-UnlinkedPath([string]$Path) {
    $absolute = [IO.Path]::GetFullPath($Path)
    $cursor = $absolute
    while ($cursor) {
        if (Test-Path -LiteralPath $cursor) {
            $item = Get-Item -LiteralPath $cursor -Force
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'NATIVE_INSTALL_LINKED_PATH' }
        }
        $parent = [IO.Path]::GetDirectoryName($cursor)
        if ($parent -eq $cursor) { break }
        $cursor = $parent
    }
    return $absolute
}
$binary = Assert-UnlinkedPath $HostBinary
$destination = Assert-UnlinkedPath $InstallRoot
$component = Assert-UnlinkedPath $ComponentDirectory
$extension = Join-Path $component 'extension'
$package = Read-NativeJson (Join-Path $component 'package.json') 'NATIVE_INSTALL_PACKAGE_INVALID'
if ($package.name -ne 'chatgpt-browser-bridge-node' -or $package.version -ne '6.4.0') { throw 'NATIVE_INSTALL_COMPONENT_VERSION' }
if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw 'NATIVE_INSTALL_BINARY_MISSING' }
$binarySha = Get-NativeFileSha $binary
$origins = [Collections.Generic.HashSet[string]]::new()
$registrations = [Collections.Generic.List[object]]::new()
foreach ($entry in @(@('Edge', $EdgeUserDataRoot), @('Chrome', $ChromeUserDataRoot))) {
    $browserName = $entry[0]
    $userRoot = $entry[1]
    if (-not (Test-Path -LiteralPath $userRoot -PathType Container)) { continue }
    $ids = [Collections.Generic.HashSet[string]]::new()
    foreach ($profile in (Get-ChildItem -LiteralPath $userRoot -Directory | Where-Object { $_.Name -eq 'Default' -or $_.Name -match '^Profile \d+$' })) {
        foreach ($filename in @('Preferences', 'Secure Preferences')) {
            $preferencesPath = Join-Path $profile.FullName $filename
            if (-not (Test-Path -LiteralPath $preferencesPath -PathType Leaf)) { continue }
            $preferences = Read-NativeJson $preferencesPath 'NATIVE_INSTALL_PREFERENCES_INVALID'
            foreach ($setting in $preferences.extensions.settings.PSObject.Properties) {
                if (-not ($setting.Value.path -is [string]) -or -not [IO.Path]::IsPathRooted($setting.Value.path)) { continue }
                if ([IO.Path]::GetFullPath($setting.Value.path).TrimEnd('\') -ine $extension.TrimEnd('\')) { continue }
                if ($setting.Name -cnotmatch '^[a-p]{32}$') { throw 'NATIVE_INSTALL_EXTENSION_ID_INVALID' }
                [void]$ids.Add($setting.Name)
                [void]$origins.Add("chrome-extension://$($setting.Name)/")
            }
        }
    }
    if ($ids.Count) {
        $vendor = if ($browserName -eq 'Edge') { 'Microsoft\Edge' } else { 'Google\Chrome' }
        $registrations.Add([ordered]@{ browser = $browserName; key = "HKCU:\Software\$vendor\NativeMessagingHosts\$hostName" })
    }
}
if (-not $origins.Count) { throw 'NATIVE_INSTALL_REGISTERED_EXTENSION_NOT_FOUND' }
$serverConfig = Join-Path $env:APPDATA 'app.director.next\chrome-connection.json'
$connection = Read-NativeJson $serverConfig 'NATIVE_INSTALL_CONNECTION_INVALID'
if ($connection.serverUrl -and $connection.serverUrl.TrimEnd('/') -ne 'http://127.0.0.1:53179') { throw 'NATIVE_INSTALL_SERVER_CONFLICT' }
# The native host never reads browser cookies or copies the bridge token to disk.
$exe = Join-Path $destination 'director-native-browser-host.exe'
$manifestPath = Join-Path $destination "$hostName.json"
$hostManifest = [ordered]@{ name = $hostName; description = 'Director browser transport'; path = $exe; type = 'stdio'; allowed_origins = @($origins | Sort-Object) }
$hostConfig = [ordered]@{ schemaVersion = 'director-native-host-config/v1'; allowedOrigins = @($origins | Sort-Object); serverUrl = 'http://127.0.0.1:53179' }
foreach ($registration in $registrations) {
    if (Test-Path -LiteralPath $registration.key) {
        $previous = (Get-Item -LiteralPath $registration.key).GetValue('')
        if ($previous -and $previous -ine $manifestPath) { throw 'NATIVE_INSTALL_FOREIGN_REGISTRATION' }
    }
}
$plan = [ordered]@{ version = 'director-native-host-install/v1'; planOnly = [bool]$PlanOnly; binarySha256 = $binarySha; manifest = $hostManifest; config = $hostConfig; registrations = $registrations.ToArray(); browserPreferencesModified = $false; policiesModified = $false }
if ($PlanOnly) { $plan | ConvertTo-Json -Depth 8; exit 0 }
New-Item -ItemType Directory -Path $destination -Force | Out-Null
$receiptPath = Join-Path $destination 'install-receipt.json'
if (Test-Path -LiteralPath $exe) {
    $installedSha = Get-NativeFileSha $exe
    if ($installedSha -ne $binarySha) {
        if (-not (Test-Path -LiteralPath $receiptPath)) { throw 'NATIVE_INSTALL_UNKNOWN_BINARY' }
        $receipt = Read-NativeJson $receiptPath 'NATIVE_INSTALL_RECEIPT_INVALID'
        if ($receipt.binarySha256 -ne $installedSha) { throw 'NATIVE_INSTALL_CHANGED_BINARY' }
        Copy-Item -LiteralPath $exe -Destination (Join-Path $destination 'director-native-browser-host-previous.exe') -Force
    }
}
foreach ($item in @(@('native-host-config.json', $hostConfig), @("$hostName.json", $hostManifest))) {
    $targetFile = Join-Path $destination $item[0]
    $json = $item[1] | ConvertTo-Json -Depth 8
    if (Test-Path -LiteralPath $targetFile) {
        $existing = Read-NativeJson $targetFile 'NATIVE_INSTALL_CONFIG_INVALID'
        if (($existing | ConvertTo-Json -Depth 8 -Compress) -ne ($item[1] | ConvertTo-Json -Depth 8 -Compress)) { throw 'NATIVE_INSTALL_CONFIG_CONFLICT' }
    } else {
        [IO.File]::WriteAllText($targetFile, $json, [Text.UTF8Encoding]::new($false))
    }
}
if (-not (Test-Path -LiteralPath $exe) -or (Get-NativeFileSha $exe) -ne $binarySha) {
    $staged = "$exe.install.tmp"
    if (Test-Path -LiteralPath $staged) { throw 'NATIVE_INSTALL_PENDING_FILE' }
    Copy-Item -LiteralPath $binary -Destination $staged
    if ((Get-NativeFileSha $staged) -ne $binarySha) { throw 'NATIVE_INSTALL_COPY_MISMATCH' }
    Move-Item -LiteralPath $staged -Destination $exe -Force
}
foreach ($registration in $registrations) {
    New-Item -Path $registration.key -Force | Out-Null
    Set-Item -LiteralPath $registration.key -Value $manifestPath
    if ((Get-Item -LiteralPath $registration.key).GetValue('') -ine $manifestPath) { throw 'NATIVE_INSTALL_REGISTRY_VERIFY' }
}
$plan.planOnly = $false
[IO.File]::WriteAllText($receiptPath, ($plan | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
$plan | ConvertTo-Json -Depth 8
