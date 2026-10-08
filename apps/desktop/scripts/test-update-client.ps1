param(
    [Parameter(Mandatory=$true)][string]$TestDirectory,
    [string]$SdkResourceDirectory
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
if (-not [System.IO.Path]::IsPathRooted($TestDirectory)) { throw '测试目录必须是新的绝对路径。' }
$testRoot = [System.IO.Path]::GetFullPath($TestDirectory)
if (Test-Path -LiteralPath $testRoot) { throw '测试目录已经存在；不复用任何现有文件。' }
if (-not $SdkResourceDirectory) { $SdkResourceDirectory = Join-Path $projectRoot 'src-tauri\resources\sdk-readiness' }
$scriptPath = Join-Path $PSScriptRoot 'update-client.ps1'
$release = Join-Path $testRoot '中文 发布目录'
$current = Join-Path $release 'director-desktop-next.exe'
$previous = Join-Path $release 'director-desktop-next-previous.exe'
New-Item -ItemType Directory -Path $testRoot | Out-Null
$candidate1 = Join-Path $testRoot 'candidate one.exe'
$candidate2 = Join-Path $testRoot 'candidate two.exe'
$candidate3 = Join-Path $testRoot 'candidate three.exe'
[System.IO.File]::WriteAllBytes($candidate1, [System.Text.Encoding]::UTF8.GetBytes('synthetic executable one; never launched'))
[System.IO.File]::WriteAllBytes($candidate2, [System.Text.Encoding]::UTF8.GetBytes('synthetic executable two; never launched'))
[System.IO.File]::WriteAllBytes($candidate3, [System.Text.Encoding]::UTF8.GetBytes('synthetic executable three; never launched'))
function Hash([string]$Path) {
    $stream = [System.IO.File]::OpenRead((NativeIoPath $Path))
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try { return [System.BitConverter]::ToString($sha256.ComputeHash($stream)).Replace('-', '').ToLowerInvariant() }
    finally { $sha256.Dispose(); $stream.Dispose() }
}
function NativeIoPath([string]$Path) {
    $absolute = [System.IO.Path]::GetFullPath($Path)
    if ($absolute.StartsWith('\\?\')) { return $absolute }
    if ($absolute.StartsWith('\\')) { return '\\?\UNC\' + $absolute.Substring(2) }
    return '\\?\' + $absolute
}
function Assert([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
$checks = [System.Collections.Generic.List[string]]::new()
$params = @{ PublishOnly=$true; ReleaseDirectory=$release; SdkResourceDirectory=$SdkResourceDirectory }
& $scriptPath @params -CandidatePath $candidate1 | Out-Null
Assert ((Hash $current) -eq (Hash $candidate1) -and -not (Test-Path -LiteralPath $previous)) '初次发布或无备用版断言失败。'
$checks.Add('first_publish_without_previous')
& $scriptPath @params -CandidatePath $candidate2 | Out-Null
Assert ((Hash $current) -eq (Hash $candidate2) -and (Hash $previous) -eq (Hash $candidate1)) '原子轮换未保留真实上一版。'
$checks.Add('atomic_rotation_preserves_previous')
foreach ($candidate in @($candidate1,$candidate2)) {
    $manifest = Join-Path (Join-Path (Join-Path $release '.client-resources') (Hash $candidate)) 'sdk-readiness\manifest.json'
    Assert ((Hash $manifest) -eq (Hash (Join-Path $SdkResourceDirectory 'manifest.json'))) '当前版或备用版SDK资源不匹配。'
}
$checks.Add('current_and_previous_resources_remain_bound')
& $scriptPath @params -CandidatePath $candidate2 | Out-Null
Assert ((Hash $previous) -eq (Hash $candidate1)) '相同版本重发不应轮换备用版。'
$checks.Add('identical_candidate_is_noop')
$clientLease = [System.IO.File]::Open($current, [System.IO.FileMode]::Open, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None)
$blocked = $false
try { try { & $scriptPath @params -CandidatePath $candidate3 | Out-Null } catch { $blocked = $true } }
finally { $clientLease.Dispose() }
Assert $blocked '使用中的客户端没有阻止轮换。'
Assert ((Hash $current) -eq (Hash $candidate2) -and (Hash $previous) -eq (Hash $candidate1)) '客户端被占用时原程序遭到修改。'
$checks.Add('in_use_client_is_preserved')
$invalidResource = Join-Path $testRoot 'tampered resources'
$sourceManifest = Get-Content -Raw -LiteralPath (Join-Path $SdkResourceDirectory 'manifest.json') | ConvertFrom-Json
foreach ($file in $sourceManifest.files) {
    $destination = Join-Path $invalidResource $file.path
    New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $SdkResourceDirectory $file.path) -Destination $destination
}
Copy-Item -LiteralPath (Join-Path $SdkResourceDirectory 'manifest.json') -Destination (Join-Path $invalidResource 'manifest.json')
[System.IO.File]::AppendAllText((Join-Path $invalidResource 'src\health.mjs'), '// deliberate synthetic tamper')
$blocked = $false
try { & $scriptPath -PublishOnly -CandidatePath $candidate3 -ReleaseDirectory $release -SdkResourceDirectory $invalidResource | Out-Null } catch { $blocked = $true }
Assert $blocked '资源篡改没有阻止发布。'
Assert ((Hash $current) -eq (Hash $candidate2) -and (Hash $previous) -eq (Hash $candidate1)) '篡改资源影响了现有程序。'
$checks.Add('tampered_resources_rejected_before_rotation')
$blocked = $false
try { & $scriptPath @params -CandidatePath $current | Out-Null } catch { $blocked = $true }
Assert $blocked '当前EXE被误当作独立候选。'
$checks.Add('public_executable_is_not_a_candidate')

# Explicit prebuilt staging must not compile or rotate either saved executable.
& $scriptPath -BuildOnly -CandidatePath $candidate3 -ReleaseDirectory $release -SdkResourceDirectory $SdkResourceDirectory | Out-Null
Assert ((Hash $current) -eq (Hash $candidate2) -and (Hash $previous) -eq (Hash $candidate1)) 'BuildOnly 改动了当前版或备用版。'
Assert ((Hash (Join-Path $release 'director-desktop-next-candidate.exe')) -eq (Hash $candidate3)) 'BuildOnly 未准备独立验收候选。'
Assert (Test-Path -LiteralPath (Join-Path (Join-Path (Join-Path $release '.client-resources') (Hash $candidate3)) 'sdk-readiness\manifest.json')) 'BuildOnly 未准备候选专属资源。'
& $scriptPath -PublishOnly -CandidatePath $candidate3 -ReleaseDirectory $release -SdkResourceDirectory $SdkResourceDirectory | Out-Null
Assert ((Hash $current) -eq (Hash $candidate3) -and (Hash $previous) -eq (Hash $candidate2)) '准备后发布没有保留正确当前版和上一轮。'
Assert (-not (Test-Path -LiteralPath (Join-Path $release 'director-desktop-next-candidate.exe'))) '成功发布留下了第三份自建候选EXE。'
Assert ((Hash $candidate3) -eq (Hash $current)) '发布清理误删或改动外部 CandidatePath。'
$checks.Add('build_only_prepares_without_rotating_entry_points')

# Copy the exact script into an isolated fake project to exercise its real
# default paths without reading or changing the task's accepted client.
$isolatedProject = Join-Path $testRoot '隔离 默认目录项目'
$isolatedScripts = Join-Path $isolatedProject 'scripts'
$isolatedTarget = Join-Path $isolatedProject 'src-tauri\target\release'
New-Item -ItemType Directory -Path $isolatedScripts -Force | Out-Null
New-Item -ItemType Directory -Path $isolatedTarget -Force | Out-Null
$isolatedScript = Join-Path $isolatedScripts 'update-client.ps1'
Copy-Item -LiteralPath $scriptPath -Destination $isolatedScript
Assert ((Hash $isolatedScript) -eq (Hash $scriptPath)) '隔离默认路径测试未使用原脚本字节。'
$targetArtifact = Join-Path $isolatedTarget 'director-desktop-next.exe'
[System.IO.File]::WriteAllBytes($targetArtifact, [System.Text.Encoding]::UTF8.GetBytes('synthetic cargo artifact; never published or launched'))
$targetHash = Hash $targetArtifact
$defaultRelease = Join-Path $isolatedProject '.local\client'
$defaultCurrent = Join-Path $defaultRelease 'director-desktop-next.exe'
& $isolatedScript -BuildOnly -CandidatePath $candidate1 -SdkResourceDirectory $SdkResourceDirectory | Out-Null
Assert (-not (Test-Path -LiteralPath $defaultCurrent) -and (Hash $targetArtifact) -eq $targetHash) '默认 BuildOnly 使用了当前入口或改动 Cargo 产物。'
Assert ((Hash (Join-Path $defaultRelease 'director-desktop-next-candidate.exe')) -eq (Hash $candidate1)) '默认验收候选未与其资源同目录准备。'
Assert (Test-Path -LiteralPath (Join-Path (Join-Path (Join-Path $defaultRelease '.client-resources') (Hash $candidate1)) 'sdk-readiness\manifest.json')) '默认发布资源没有准备到独立 client 目录。'
$checks.Add('default_build_only_uses_isolated_client_directory')
$defaultPrepared = Join-Path $defaultRelease 'director-desktop-next-candidate.exe'
$preparedLease = [System.IO.File]::Open($defaultPrepared, [System.IO.FileMode]::Open, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None)
try { $publishMessages = @(& $isolatedScript -PublishOnly -CandidatePath $candidate1 -SdkResourceDirectory $SdkResourceDirectory) }
finally { $preparedLease.Dispose() }
Assert ((Hash $defaultCurrent) -eq (Hash $candidate1) -and (Hash $targetArtifact) -eq $targetHash) '默认 PublishOnly 改动了 Cargo 产物或没有发布独立当前入口。'
Assert ((Test-Path -LiteralPath $defaultPrepared) -and ($publishMessages -match '验收候选仍在使用').Count -ge 1) '使用中的候选未保留或没有说明。'
& $isolatedScript -PublishOnly -CandidatePath $candidate1 -SdkResourceDirectory $SdkResourceDirectory | Out-Null
Assert (-not (Test-Path -LiteralPath $defaultPrepared) -and -not (Test-Path -LiteralPath (Join-Path $defaultRelease '.prepared-candidate.json'))) '候选关闭后重发没有窄清理自建候选。'
Assert ((Hash $candidate1) -eq (Hash $defaultCurrent) -and (Hash $targetArtifact) -eq $targetHash) '候选清理改动了外部文件或 Cargo 产物。'
$checks.Add('default_publish_preserves_cargo_target_artifact')
$blocked = $false
try { & $isolatedScript -PublishOnly -CandidatePath $candidate2 -ReleaseDirectory $isolatedTarget -SdkResourceDirectory $SdkResourceDirectory | Out-Null } catch { $blocked = $true }
Assert ($blocked -and (Hash $targetArtifact) -eq $targetHash -and (Hash $defaultCurrent) -eq (Hash $candidate1)) '错误的 Cargo 发布目录未被阻断。'
$checks.Add('cargo_target_is_never_a_publish_directory')

# Force the real SDK reader/copy path beyond MAX_PATH on every checkout. Each
# component stays below the per-component limit; no fixture root is shortened.
$longSdk = Join-Path (Join-Path (Join-Path $testRoot ('sdk-long-' + ('a' * 80))) ('nested-' + ('b' * 80))) ('nested-' + ('c' * 80))
$null = [System.IO.Directory]::CreateDirectory((NativeIoPath $longSdk))
foreach ($file in $sourceManifest.files) {
    $destination = Join-Path $longSdk $file.path
    $null = [System.IO.Directory]::CreateDirectory((NativeIoPath (Split-Path -Parent $destination)))
    [System.IO.File]::Copy((NativeIoPath (Join-Path $SdkResourceDirectory $file.path)), (NativeIoPath $destination))
}
[System.IO.File]::Copy((NativeIoPath (Join-Path $SdkResourceDirectory 'manifest.json')), (NativeIoPath (Join-Path $longSdk 'manifest.json')))
$longestSdkPath = ($sourceManifest.files | ForEach-Object { (Join-Path $longSdk $_.path).Length } | Measure-Object -Maximum).Maximum
Assert ($longSdk.Length -gt 260 -and $longestSdkPath -gt 260) '长路径回归未真正越过 MAX_PATH。'
$longRelease = Join-Path $testRoot 'long-sdk-client'
$longCurrent = Join-Path $longRelease 'director-desktop-next.exe'
$longPrevious = Join-Path $longRelease 'director-desktop-next-previous.exe'
foreach ($candidate in @($candidate1,$candidate2)) {
    & $scriptPath -PublishOnly -CandidatePath $candidate -ReleaseDirectory $longRelease -SdkResourceDirectory $longSdk | Out-Null
}
Assert ((Hash $longCurrent) -eq (Hash $candidate2) -and (Hash $longPrevious) -eq (Hash $candidate1)) '长 SDK 路径改变了当前版/上一版轮换。'
foreach ($file in $sourceManifest.files) {
    $published = Join-Path (Join-Path (Join-Path $longRelease '.client-resources') (Hash $candidate2)) ('sdk-readiness\' + $file.path)
    Assert ((Hash $published) -eq $file.sha256) '长路径 SDK 未逐字发布。'
}
$checks.Add('long_sdk_native_read_copy_and_atomic_rotation')
[System.IO.File]::AppendAllText((NativeIoPath (Join-Path $longSdk 'src\health.mjs')), '// deliberate long-path tamper')
$blocked = $false
try { & $scriptPath -PublishOnly -CandidatePath $candidate3 -ReleaseDirectory $longRelease -SdkResourceDirectory $longSdk | Out-Null } catch { $blocked = $true }
Assert ($blocked -and (Hash $longCurrent) -eq (Hash $candidate2) -and (Hash $longPrevious) -eq (Hash $candidate1)) '长路径资源篡改没有保留当前版和上一版。'
$checks.Add('long_sdk_tamper_preserves_current_and_previous')
$sdkJunction = Join-Path $testRoot 'sdk-junction'
New-Item -ItemType Junction -Path $sdkJunction -Target $SdkResourceDirectory | Out-Null
$blocked = $false
try { & $scriptPath -PublishOnly -CandidatePath $candidate3 -ReleaseDirectory $longRelease -SdkResourceDirectory $sdkJunction | Out-Null } catch { $blocked = $true }
Assert ($blocked -and (Hash $longCurrent) -eq (Hash $candidate2) -and (Hash $longPrevious) -eq (Hash $candidate1)) 'extended I/O 放宽了 SDK 重解析点保护。'
$checks.Add('native_io_preserves_sdk_reparse_point_rejection')

$receipt = [ordered]@{
    schemaVersion='windows-client-update-synthetic/v1'
    platform=[System.Environment]::OSVersion.VersionString
    passed=$checks.Count
    failed=0
    checks=$checks
    updateScriptSha256=(Hash $scriptPath)
    sourceResourceManifestSha256=(Hash (Join-Path $SdkResourceDirectory 'manifest.json'))
    originalChecks=11
    longSdkRootCharacters=$longSdk.Length
    longestSdkResourcePathCharacters=$longestSdkPath
    syntheticExecutablesLaunched=0
    realGenerationCalls=0
    productionProjectsAccessed=$false
    originalClientReplaced=$false
}
$receipt | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $testRoot 'receipt.json') -Encoding utf8
Write-Output "Windows client update synthetic checks: $($checks.Count) passed; 0 failed. Receipt: $(Join-Path $testRoot 'receipt.json')"
