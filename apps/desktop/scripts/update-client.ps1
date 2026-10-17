param(
    [switch]$BuildOnly,
    [switch]$PublishOnly,
    [string]$CandidatePath,
    [string]$ReleaseDirectory,
    [string]$SdkResourceDirectory
)
$ErrorActionPreference = 'Stop'
if ($BuildOnly -and $PublishOnly) { throw 'BuildOnly 与 PublishOnly 不能同时使用。' }
$projectRoot = Split-Path -Parent $PSScriptRoot
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $projectRoot '..\..'))
if (-not $ReleaseDirectory) { $ReleaseDirectory = Join-Path $projectRoot '.local\client' }
if (-not $SdkResourceDirectory) { $SdkResourceDirectory = Join-Path $projectRoot 'src-tauri\resources\sdk-readiness' }
$releaseRoot = [System.IO.Path]::GetFullPath($ReleaseDirectory)
$cargoTargetRoot = [System.IO.Path]::GetFullPath((Join-Path $projectRoot 'src-tauri\target'))
if ($releaseRoot.Equals($cargoTargetRoot, [System.StringComparison]::OrdinalIgnoreCase) -or $releaseRoot.StartsWith($cargoTargetRoot + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw '发布目录必须独立于 Cargo 构建目录；构建产物不得充当当前版或备用版。'
}
$currentPath = Join-Path $releaseRoot 'director-desktop-next.exe'
$previousPath = Join-Path $releaseRoot 'director-desktop-next-previous.exe'
$stagedPath = Join-Path $releaseRoot 'director-desktop-next.update.tmp'
$preparedCandidatePath = Join-Path $releaseRoot 'director-desktop-next-candidate.exe'
$preparedCandidateReceiptPath = Join-Path $releaseRoot '.prepared-candidate.json'

function Get-NativeIoPath([string]$Path) {
    # Keep all containment checks on ordinary canonical paths. Only .NET I/O
    # receives a verbatim path: Windows PowerShell 5.1's provider rejects
    # existing SDK files beyond MAX_PATH, even when native I/O can read them.
    $absolute = [System.IO.Path]::GetFullPath($Path)
    if ($absolute.StartsWith('\\?\')) { return $absolute }
    if ($absolute.StartsWith('\\')) { return '\\?\UNC\' + $absolute.Substring(2) }
    return '\\?\' + $absolute
}
function Get-ClientHash([string]$Path) {
    $stream = [System.IO.File]::OpenRead((Get-NativeIoPath $Path))
    $hasher = [System.Security.Cryptography.SHA256]::Create()
    try { return [System.BitConverter]::ToString($hasher.ComputeHash($stream)).Replace('-', '').ToLowerInvariant() }
    finally { $hasher.Dispose(); $stream.Dispose() }
}
function Get-VerifiedSdkManifest([string]$Directory) {
    $root = [System.IO.Path]::GetFullPath($Directory)
    if (-not [System.IO.Directory]::Exists((Get-NativeIoPath $root))) { throw '缺少已准备的 SDK 只读资源。' }
    $manifestPath = Join-Path $root 'manifest.json'
    $manifest = [System.IO.File]::ReadAllText((Get-NativeIoPath $manifestPath)) | ConvertFrom-Json
    $allowed = @('src/health.mjs','src/bridge.mjs','src/main.mjs','src/native-runtime.mjs','THIRD_PARTY_NOTICES.md','package-lock.json','node_modules/@openai/codex-sdk/package.json','node_modules/@openai/codex-sdk/dist/index.js','node_modules/@openai/codex-sdk/LICENSE')
    if ($manifest.schemaVersion -ne 'sdk-readiness-resources/v1' -or $manifest.sdkPackage -ne '@openai/codex-sdk' -or $manifest.sdkVersion -ne '0.160.1' -or $manifest.officialRuntimeIncluded -ne $false -or $manifest.modelCalls -ne 0 -or $manifest.files.Count -ne $allowed.Count) {
        throw 'SDK 资源清单身份无效；未轮换客户端。'
    }
    $seen = @{}
    foreach ($file in $manifest.files) {
        if ($allowed -notcontains $file.path -or $seen.ContainsKey($file.path)) { throw 'SDK 资源路径不在固定清单内。' }
        $seen[$file.path] = $true
        $itemPath = [System.IO.Path]::GetFullPath((Join-Path $root $file.path))
        if (-not $itemPath.StartsWith($root + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) { throw 'SDK 资源路径越界。' }
        $item = [System.IO.FileInfo]::new((Get-NativeIoPath $itemPath))
        if (-not $item.Exists -or ($item.Attributes -band [System.IO.FileAttributes]::Directory) -or $item.Length -gt 2097152 -or $item.Length -ne $file.bytes -or (Get-ClientHash $itemPath) -ne $file.sha256) { throw 'SDK 资源字节验证失败。' }
        for ($ancestor = $itemPath; $ancestor -and $ancestor.Length -ge $root.Length; $ancestor = Split-Path -Parent $ancestor) {
            if ([System.IO.File]::GetAttributes((Get-NativeIoPath $ancestor)) -band [System.IO.FileAttributes]::ReparsePoint) { throw 'SDK 资源不得通过符号链接发布。' }
            if ($ancestor -eq $root) { break }
        }
    }
    $pins = @{
        'node_modules/@openai/codex-sdk/package.json' = '5ffdee6d6135684e502d2d6010bae3690c0e2acb44e95d1e27c14fc5bbfccf30'
        'node_modules/@openai/codex-sdk/dist/index.js' = 'd62ed107033bdba802b283c77d875e4bec3deb2704a910bb7e3f95059473b16f'
        'node_modules/@openai/codex-sdk/LICENSE' = 'd17f227e4df5da1600391338865ce0f3055211760a36688f816941d58232d8dc'
    }
    foreach ($entry in $pins.GetEnumerator()) {
        if ((Get-ClientHash (Join-Path $root $entry.Key)) -ne $entry.Value) { throw 'SDK 官方包与固定版本不符。' }
    }
    return $manifest
}

function Remove-PublishedPreparedCandidate([string]$PublishedHash) {
    # Only a byte-matching candidate created by this script in this client
    # directory can be removed. Caller-selected external paths are never used.
    if (-not (Test-Path -LiteralPath $preparedCandidatePath -PathType Leaf)) { return }
    if (-not (Test-Path -LiteralPath $preparedCandidateReceiptPath -PathType Leaf)) {
        Write-Output '候选没有本脚本的准备记录，已保留；当前版和备用版已发布。'
        return
    }
    foreach ($itemPath in @($preparedCandidatePath, $preparedCandidateReceiptPath)) {
        if ((Get-Item -LiteralPath $itemPath).Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
            Write-Output '候选或准备记录是重解析点，已保留；当前版和备用版已发布。'
            return
        }
    }
    try { $prepared = Get-Content -Raw -LiteralPath $preparedCandidateReceiptPath | ConvertFrom-Json }
    catch { Write-Output '候选准备记录无法核验，已保留；当前版和备用版已发布。'; return }
    if ($prepared.schemaVersion -ne 'windows-client-prepared-candidate/v1' -or $prepared.fileName -ne 'director-desktop-next-candidate.exe' -or $prepared.sha256 -ne $PublishedHash) { return }
    $candidateLease = $null
    try { $candidateLease = [System.IO.File]::Open($preparedCandidatePath, [System.IO.FileMode]::Open, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None) }
    catch { Write-Output '验收候选仍在使用，已保留；自然关闭后再次 PublishOnly 可清理，当前版和备用版已发布。'; return }
    try {
        $sha256 = [System.Security.Cryptography.SHA256]::Create()
        try { $actualHash = [System.BitConverter]::ToString($sha256.ComputeHash($candidateLease)).Replace('-', '').ToLowerInvariant() }
        finally { $sha256.Dispose() }
        if ($actualHash -ne $PublishedHash -or (Get-ClientHash $currentPath) -ne $PublishedHash) { return }
    } finally { $candidateLease.Dispose() }
    try {
        Remove-Item -LiteralPath $preparedCandidatePath
        Remove-Item -LiteralPath $preparedCandidateReceiptPath
    } catch { Write-Output '验收候选或准备记录仍被占用，未强制清理；当前版和备用版已发布。' }
}

New-Item -ItemType Directory -Path $releaseRoot -Force | Out-Null
try { $updateLease = [System.IO.File]::Open((Join-Path $releaseRoot '.client-update.lock'), [System.IO.FileMode]::OpenOrCreate, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None) }
catch { throw '此目录已有更新正在执行，请等待该次更新完成。' }
try {
    # A caller-provided prebuilt candidate can be staged without a new compiler
    # invocation. Only PublishOnly can rotate the current/previous entry points.
    if (-not $PublishOnly -and -not $CandidatePath) {
        Push-Location $projectRoot
        try {
            & node (Join-Path $repositoryRoot 'sdk-bridge\scripts\package-readiness.mjs')
            if ($LASTEXITCODE -ne 0) { throw 'SDK 只读资源准备失败，当前版与备用版未更改。' }
            & node (Join-Path $repositoryRoot 'web-image-bridge\package-runtime.mjs')
            if ($LASTEXITCODE -ne 0) { throw 'OpenCLI 网页组件准备失败，当前版与备用版未更改。' }
            & npm run build
            if ($LASTEXITCODE -ne 0) { throw '界面构建失败，当前版与备用版未更改。' }
            $cargo = Get-Command cargo -ErrorAction SilentlyContinue
            $cargoPath = if ($cargo) { $cargo.Source } else { Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }
            & $cargoPath rustc --locked --manifest-path src-tauri/Cargo.toml --target-dir $cargoTargetRoot --release --features desktop,custom-protocol --bin director-desktop-next -- -C extra-filename=-candidate
            if ($LASTEXITCODE -ne 0) { throw '客户端构建失败，当前版与备用版未更改。' }
            $CandidatePath = Join-Path $cargoTargetRoot 'release\deps\director_desktop_next-candidate.exe'
        } finally { Pop-Location }
    }
    if (-not $CandidatePath -or -not (Test-Path -LiteralPath $CandidatePath -PathType Leaf)) { throw '请提供独立候选程序。' }
    $candidate = (Resolve-Path -LiteralPath $CandidatePath).Path
    if ($candidate -eq $currentPath -or $candidate -eq $previousPath -or $candidate -eq $stagedPath) { throw '候选程序必须独立于当前版和备用版。' }
    $candidateHash = Get-ClientHash $candidate
    $manifest = Get-VerifiedSdkManifest $SdkResourceDirectory
    $ownedResources = Join-Path $releaseRoot '.client-resources'
    New-Item -ItemType Directory -Path $ownedResources -Force | Out-Null
    foreach ($ownedDirectory in @($releaseRoot, $ownedResources)) {
        if ((Get-Item -LiteralPath $ownedDirectory).Attributes -band [System.IO.FileAttributes]::ReparsePoint) { throw '客户端发布目录不得是符号链接或重解析点。' }
    }
    $resourceRoot = Join-Path $ownedResources $candidateHash
    $publishedSdk = Join-Path $resourceRoot 'sdk-readiness'
    if (Test-Path -LiteralPath $resourceRoot) {
        $existing = Get-VerifiedSdkManifest $publishedSdk
        if ((Get-ClientHash (Join-Path $publishedSdk 'manifest.json')) -ne (Get-ClientHash (Join-Path $SdkResourceDirectory 'manifest.json'))) { throw '相同客户端SHA已有不同资源；未覆盖。' }
    } else {
        $resourceStage = Join-Path $ownedResources ('.staging-' + [System.Guid]::NewGuid().ToString('N'))
        try {
            $stageSdk = Join-Path $resourceStage 'sdk-readiness'
            New-Item -ItemType Directory -Path $stageSdk -Force | Out-Null
            foreach ($file in $manifest.files) {
                $destination = Join-Path $stageSdk $file.path
                $null = [System.IO.Directory]::CreateDirectory((Get-NativeIoPath (Split-Path -Parent $destination)))
                [System.IO.File]::Copy((Get-NativeIoPath (Join-Path $SdkResourceDirectory $file.path)), (Get-NativeIoPath $destination))
            }
            [System.IO.File]::Copy((Get-NativeIoPath (Join-Path $SdkResourceDirectory 'manifest.json')), (Get-NativeIoPath (Join-Path $stageSdk 'manifest.json')))
            $null = Get-VerifiedSdkManifest $stageSdk
            $opencliSource = Join-Path $projectRoot 'src-tauri\resources\opencli'
            if (Test-Path -LiteralPath (Join-Path $opencliSource 'runtime-manifest.json') -PathType Leaf) {
                $opencliManifest = Get-Content -Raw -LiteralPath (Join-Path $opencliSource 'runtime-manifest.json') | ConvertFrom-Json
                if ($opencliManifest.version -ne 'director-opencli-runtime/v1') { throw 'OpenCLI 组件清单版本无效。' }
                $opencliStage = Join-Path $resourceStage 'opencli'
                foreach ($file in $opencliManifest.files) {
                    $sourceFile = [IO.Path]::GetFullPath((Join-Path $opencliSource $file.path))
                    $targetFile = [IO.Path]::GetFullPath((Join-Path $opencliStage $file.path))
                    if (-not $sourceFile.StartsWith([IO.Path]::GetFullPath($opencliSource) + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or -not $targetFile.StartsWith([IO.Path]::GetFullPath($opencliStage) + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'OpenCLI 资源路径越界。' }
                    if ((Get-ClientHash $sourceFile) -ne $file.sha256) { throw 'OpenCLI 资源字节校验失败。' }
                    $null = [IO.Directory]::CreateDirectory((Get-NativeIoPath (Split-Path -Parent $targetFile)))
                    [IO.File]::Copy((Get-NativeIoPath $sourceFile), (Get-NativeIoPath $targetFile))
                }
                [IO.File]::Copy((Join-Path $opencliSource 'runtime-manifest.json'), (Join-Path $opencliStage 'runtime-manifest.json'))
            }
            [System.IO.Directory]::Move($resourceStage, $resourceRoot)
        } finally {
            if (Test-Path -LiteralPath $resourceStage) {
                $checkedStage = [System.IO.Path]::GetFullPath($resourceStage)
                $checkedParent = [System.IO.Path]::GetFullPath((Join-Path $releaseRoot '.client-resources'))
                if (-not $checkedStage.StartsWith($checkedParent + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase) -or -not (Split-Path -Leaf $checkedStage).StartsWith('.staging-')) { throw '拒绝清理发布目录以外的临时资源。' }
                $cleanupItems = @((Get-Item -LiteralPath $checkedStage)) + @(Get-ChildItem -LiteralPath $checkedStage -Recurse -Force)
                if ($cleanupItems | Where-Object { $_.Attributes -band [System.IO.FileAttributes]::ReparsePoint }) { throw '临时资源出现重解析点；未进行递归清理。' }
                Remove-Item -LiteralPath $checkedStage -Recurse -Force
            }
        }
    }
    if ($BuildOnly -or -not $PublishOnly) {
        # This candidate sits beside its SHA-bound resources so native checks
        # do not depend on the compiler output directory or a source checkout.
        if ($candidate -ne $preparedCandidatePath) {
            Copy-Item -LiteralPath $candidate -Destination $preparedCandidatePath -Force
            if ((Get-ClientHash $preparedCandidatePath) -ne $candidateHash) { throw '独立验收候选复制校验失败。' }
            $preparedReceipt = [ordered]@{ schemaVersion='windows-client-prepared-candidate/v1'; fileName='director-desktop-next-candidate.exe'; sha256=$candidateHash }
            [System.IO.File]::WriteAllText($preparedCandidateReceiptPath, ($preparedReceipt | ConvertTo-Json -Compress), [System.Text.UTF8Encoding]::new($false))
        }
        if ((Get-ClientHash $preparedCandidatePath) -ne $candidateHash) { throw '独立验收候选复制校验失败。' }
        Write-Output "已准备独立候选：$preparedCandidatePath"
        Write-Output "候选SHA256：$candidateHash"
        Write-Output "SDK资源：$resourceRoot"
        return
    }
    if ((Test-Path -LiteralPath $currentPath) -and (Get-ClientHash $currentPath) -eq $candidateHash) {
        Remove-PublishedPreparedCandidate $candidateHash
        Write-Output '当前程序已是此版本，备用版保持不变。'
        return
    }
    $leases = @()
    try {
        foreach ($client in @($currentPath, $previousPath)) {
            if (Test-Path -LiteralPath $client) {
                try { $leases += [System.IO.File]::Open($client, [System.IO.FileMode]::Open, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None) }
                catch { throw '当前版或备用版正在使用，请自然关闭后再更新；两份程序均未轮换。' }
            }
        }
    } finally { foreach ($lease in $leases) { $lease.Dispose() } }
    try {
        Copy-Item -LiteralPath $candidate -Destination $stagedPath -Force
        if ((Get-ClientHash $stagedPath) -ne $candidateHash) { throw '新程序复制校验失败。' }
        if (Test-Path -LiteralPath $currentPath) { [System.IO.File]::Replace($stagedPath, $currentPath, $previousPath) }
        else { [System.IO.File]::Move($stagedPath, $currentPath) }
        if ((Get-ClientHash $currentPath) -ne $candidateHash) { throw '发布后程序校验失败，请保留备用版。' }
        Remove-PublishedPreparedCandidate $candidateHash
        Write-Output '已更新独立 director-desktop-next.exe；上一轮保留为 director-desktop-next-previous.exe。SDK资源按各自程序SHA隔离。'
    } finally { if (Test-Path -LiteralPath $stagedPath) { Remove-Item -LiteralPath $stagedPath } }
} finally { $updateLease.Dispose() }
