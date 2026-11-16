#requires -Version 5.1
[CmdletBinding()]
param(
    [ValidatePattern('^[0-9A-Za-z][0-9A-Za-z.+-]{0,60}$')][string]$Version,
    [string]$CatalogBundle,
    [switch]$SkipWebBuild
)
$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$cargo=Get-Command cargo.exe -ErrorAction Stop
$cargoText=Get-Content -LiteralPath (Join-Path $root 'Cargo.toml') -Raw
$canonical=[regex]::Match($cargoText,'(?ms)^\[workspace.package\]\s*version\s*=\s*"([^"]+)"').Groups[1].Value
if (!$canonical) { throw 'Workspace release version missing' }
if (!$Version) { $Version=$canonical }
if ($Version -ne $canonical) { throw 'Portable version must equal workspace.package.version' }
$buildCommit=(& git -C $root rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Build commit unavailable' }
$dirty=& git -C $root status --porcelain --untracked-files=all
if ($LASTEXITCODE -ne 0 -or $dirty) { throw 'Release build requires a clean checkout with all source changes committed' }
$oldBuildCommit=$env:VISION_BUILD_COMMIT
$env:VISION_BUILD_COMMIT=$buildCommit
$packageName="VisionOverlay-$Version-windows-x64-portable"
$dist=Join-Path $root 'dist'
$destination=Join-Path $dist $packageName
if ((Test-Path -LiteralPath $destination) -or (Test-Path -LiteralPath ($destination+'.zip'))) { throw 'Output already exists. Choose a new version; release folders are never overwritten.' }
$runtimeVersion='3.14.10'
$runtimeHash='b553e7dac884469b40e0740bb527271c3378f700b427e0485835f1bb6424361c'
$runtimeUrl="https://www.python.org/ftp/python/$runtimeVersion/python-$runtimeVersion-embeddable-amd64.zip"
$cache=Join-Path $root 'local-data/build-cache'
New-Item -ItemType Directory -Path $cache -Force | Out-Null
$runtimeZip=Join-Path $cache "python-$runtimeVersion-embeddable-amd64.zip"
if (!(Test-Path -LiteralPath $runtimeZip)) { Invoke-WebRequest -UseBasicParsing -Uri $runtimeUrl -OutFile $runtimeZip }
if ((Get-FileHash -LiteralPath $runtimeZip -Algorithm SHA256).Hash.ToLowerInvariant() -ne $runtimeHash) { throw 'Python runtime SHA256 mismatch; cached file was not executed.' }
Push-Location $root
$oldFlags=$env:RUSTFLAGS
try {
    $env:RUSTFLAGS='-C target-feature=+crt-static'
    $CatalogBundle=& (Join-Path $PSScriptRoot 'Prepare-Catalog.ps1') -CatalogBundle $CatalogBundle
    & $cargo.Source run --release --locked --target x86_64-pc-windows-msvc --target-dir target/portable -p app-server --bin catalog-package -- --input $CatalogBundle --output (Join-Path $root 'local-data/build-input/catalog.payload.json') --references-dir (Join-Path $root 'local-data/build-input/reference')
    if ($LASTEXITCODE -ne 0) { throw 'Portable catalog validation failed' }
    if (!$SkipWebBuild) { & (Join-Path $PSScriptRoot 'Build-Web.ps1') -PreparedCatalog }
    Copy-Item -LiteralPath (Join-Path $root 'local-data/build-input/catalog.payload.json') -Destination (Join-Path $root 'web/dist/catalog.json') -Force
    & $cargo.Source build --release --locked --target x86_64-pc-windows-msvc --target-dir target/portable -p app-server -p capture-probe -p vision-launcher --bin vision-server --bin capture-probe --bin VisionOverlay
    if ($LASTEXITCODE -ne 0) { throw 'Release compilation failed' }
    $bin=Join-Path $root 'target/portable/x86_64-pc-windows-msvc/release'
    New-Item -ItemType Directory -Path (Join-Path $destination 'components/scripts'),(Join-Path $destination 'components/reference'),(Join-Path $destination 'components/python'),(Join-Path $destination 'licenses') -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $bin 'VisionOverlay.exe') -Destination $destination
    foreach ($name in 'vision-server.exe','capture-probe.exe') { Copy-Item -LiteralPath (Join-Path $bin $name) -Destination (Join-Path $destination 'components') }
    foreach ($name in 'Runtime.Paths.ps1','Configure-AutoCapture.ps1','Watch-GameCapture.ps1','AutoCapture.State.ps1','Start-RealtimeCapture.ps1','Test-CaptureEnvironment.ps1','sync_wish_history.py','import_wish_history.py') { Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination (Join-Path $destination 'components/scripts') }
    $dispatch=Join-Path $root 'packaging/dispatch/gi.json'
    $dispatchHash='e0e1fcbfb6aa5d727367a60574b7688a4da14abe12c5a3bdad3a7fc87c694d18'
    if ((Get-FileHash -LiteralPath $dispatch -Algorithm SHA256).Hash.ToLowerInvariant() -ne $dispatchHash) { throw 'Bundled dispatch file does not match the pinned upstream version' }
    Copy-Item -LiteralPath $dispatch -Destination (Join-Path $destination 'components/reference/gi.json')
    Copy-Item -LiteralPath (Join-Path $root 'packaging/dispatch/source.json') -Destination (Join-Path $destination 'components/reference/dispatch-source.json')
    Copy-Item -LiteralPath (Join-Path $root 'packaging/dispatch/Irminsul-LICENSE.txt') -Destination (Join-Path $destination 'licenses/Irminsul-LICENSE.txt')
    foreach ($name in 'materials.ru.json','avatars.ru.json','achievements.ru.json') { Copy-Item -LiteralPath (Join-Path $root ('local-data/build-input/reference/'+$name)) -Destination (Join-Path $destination 'components/reference') }
    Expand-Archive -LiteralPath $runtimeZip -DestinationPath (Join-Path $destination 'components/python')
    # Isolated Python ignores system installations, PYTHONPATH and site-packages.
    [IO.File]::WriteAllText((Join-Path $destination 'components/python/python314._pth'),"python314.zip`n.`n../scripts`n",[Text.UTF8Encoding]::new($false))
    $manifest=@{schema_version=1;version=$Version;build_commit=$buildCommit}
    [IO.File]::WriteAllText((Join-Path $destination 'portable.json'),($manifest | ConvertTo-Json),[Text.UTF8Encoding]::new($false))
    Copy-Item -LiteralPath (Join-Path $root 'packaging/README.ru.txt') -Destination (Join-Path $destination 'README.txt')
    Copy-Item -LiteralPath (Join-Path $root 'LICENSE') -Destination (Join-Path $destination 'licenses/VisionOverlay-LICENSE.txt')
    Copy-Item -LiteralPath (Join-Path $root 'web/public/fonts/Nunito-OFL.txt') -Destination (Join-Path $destination 'licenses')
    Copy-Item -LiteralPath (Join-Path $root 'THIRD_PARTY_NOTICES.md') -Destination (Join-Path $destination 'licenses')
    Copy-Item -LiteralPath (Join-Path $root 'packaging/catalog/source.json') -Destination (Join-Path $destination 'components/reference/catalog-source.json')
    Copy-Item -LiteralPath (Join-Path $root 'web/src/data/PAIMON_LICENSE.txt') -Destination (Join-Path $destination 'licenses')
    foreach ($name in 'react','react-dom','scheduler') { Copy-Item -LiteralPath (Join-Path $root "web/node_modules/$name/LICENSE") -Destination (Join-Path $destination "licenses/$name-LICENSE.txt") }
    $metadata=(& $cargo.Source metadata --locked --format-version 1 | ConvertFrom-Json)
    if ($LASTEXITCODE -ne 0) { throw 'Dependency license enumeration failed' }
    $index=@()
    foreach ($pkg in $metadata.packages) {
        $folder=Split-Path $pkg.manifest_path -Parent
        $target=Join-Path $destination ('licenses/rust/'+$pkg.name+'-'+$pkg.version)
        $licenses=@(Get-ChildItem -LiteralPath $folder -File | Where-Object {$_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE|COPYRIGHT)($|[._-])'})
        if (!$licenses.Count) {
            $override=Join-Path $root ('packaging/license-overrides/'+$pkg.name+'-'+$pkg.version)
            if (Test-Path -LiteralPath $override) { $licenses=@(Get-ChildItem -LiteralPath $override -File) }
            elseif ($pkg.manifest_path.StartsWith((Join-Path $root 'crates')+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) { $licenses=@(Get-Item -LiteralPath (Join-Path $root 'LICENSE')) }
            else { throw ('No license notices found for '+$pkg.name+' '+$pkg.version) }
        }
        if ($licenses.Count) { New-Item -ItemType Directory -Path $target -Force | Out-Null;foreach($file in $licenses){Copy-Item -LiteralPath $file.FullName -Destination $target} }
        $index+=@{name=$pkg.name;version=$pkg.version;license=$pkg.license;repository=$pkg.repository;notices=@($licenses | ForEach-Object {$_.Name})}
    }
    [IO.File]::WriteAllText((Join-Path $destination 'licenses/rust-dependencies.json'),($index | ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $destination 'licenses/DATA-NOTICE.txt'),'Game names, reference data and remotely fetched artwork belong to their respective owners. The public dispatch file keys/gi.json from konkers/irminsul is bundled at a pinned revision; provenance is in components/reference/dispatch-source.json and the upstream license in licenses/Irminsul-LICENSE.txt. No game executable, game metadata, account credentials or user data are bundled. Reference-data source revision is available in Settings / Catalog. This build is an unofficial companion.',[Text.UTF8Encoding]::new($false))
    $hashes=@(Get-ChildItem -LiteralPath $destination -File -Recurse | ForEach-Object {@{path=$_.FullName.Substring($destination.Length+1).Replace('\','/');bytes=$_.Length;sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}})
    [IO.File]::WriteAllText((Join-Path $destination 'package-manifest.json'),(@{schema_version=1;version=$Version;platform='windows-x64';build_commit=$buildCommit;python=@{version=$runtimeVersion;url=$runtimeUrl;sha256=$runtimeHash};files=$hashes} | ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
    # Some source archives have timestamps outside the ZIP DOS date range.
    foreach ($file in Get-ChildItem -LiteralPath $destination -File -Recurse) {
        if ($file.LastWriteTimeUtc.Year -lt 1980 -or $file.LastWriteTimeUtc.Year -gt 2107) { $file.LastWriteTimeUtc=[DateTime]::SpecifyKind([DateTime]'1980-01-01',[DateTimeKind]::Utc) }
    }
    & (Join-Path $destination 'components/python/python.exe') (Join-Path $PSScriptRoot 'package_portable.py') $destination ($destination+'.zip')
    if ($LASTEXITCODE -ne 0) { throw 'Canonical portable ZIP creation failed' }
    $hash=(Get-FileHash -LiteralPath ($destination+'.zip') -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText(($destination+'.zip.sha256'),($hash+'  '+$packageName+".zip`n"),[Text.UTF8Encoding]::new($false))
    Write-Output "Portable ZIP: $destination.zip"
    Write-Output "SHA256: $hash"
} finally { $env:RUSTFLAGS=$oldFlags;$env:VISION_BUILD_COMMIT=$oldBuildCommit;Pop-Location }
