# Disposable hosted Windows T24-29 proof; never attach or format a volume.
param([switch]$CollectOnly)
$ErrorActionPreference = 'Stop'
$suffix = "dendro-foundation-$env:GITHUB_RUN_ID-$env:GITHUB_RUN_ATTEMPT"
$receipts = Join-Path $env:RUNNER_TEMP "$suffix-receipts"
if (-not (Test-Path -LiteralPath $receipts)) { New-Item -ItemType Directory -Path $receipts | Out-Null }
function Write-FixtureVerdict {
    $code = $null
    $exitPath = Join-Path $receipts 'native-fixtures.exit'
    if (Test-Path -LiteralPath $exitPath) {
        $parsed = 0
        if ([int]::TryParse((Get-Content -LiteralPath $exitPath -Raw).Trim(), [ref]$parsed)) { $code = $parsed }
    }
    $logPath = Join-Path $receipts 'native-fixtures.log'
    $log = if (Test-Path -LiteralPath $logPath) { Get-Content -LiteralPath $logPath -Raw } else { '' }
    $markers = $log.Contains('T24-29 actual distinct-device copy, private destination, unchanged closed source and verified local pointer: passed') -and $log.Contains('1 passed; 0 failed')
    $passed = ($null -ne $code -and $code -eq 0 -and $markers)
    @{
        schema = 'dendro.foundation-windows-fixture-verdict/v1'; passed = $passed
        status = $(if ($passed) { 'passed' } elseif ($null -eq $code) { 'not_run_or_interrupted' } else { 'failed' })
        candidate_commit = $env:GITHUB_SHA; exit_code = $code; positive_markers = $markers
        scope = 'native Windows cross-volume fixture only; not full foundation or release acceptance'
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $receipts 'verdict.json')
}
if ($CollectOnly) { Write-FixtureVerdict; return }
$status = 1
try {
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_OS -ne 'Windows') { throw 'Hosted Windows fixture only' }
$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
function New-PrivateDirectory([string]$Path) {
    if (Test-Path -LiteralPath $Path) { throw "Fixture already exists: $Path" }
    $acl = [System.Security.AccessControl.DirectorySecurity]::new()
    $acl.SetOwner($sid)
    $acl.SetAccessRuleProtection($true, $false)
    foreach ($trusted in @($sid.Value, 'S-1-5-18', 'S-1-5-32-544')) {
        $rule = [System.Security.AccessControl.FileSystemAccessRule]::new(
            [System.Security.Principal.SecurityIdentifier]::new($trusted),
            'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow')
        $acl.AddAccessRule($rule)
    }
    [System.IO.FileSystemAclExtensions]::Create([System.IO.DirectoryInfo]::new($Path), $acl)
    $actual = Get-Acl -LiteralPath $Path
    if (-not $actual.AreAccessRulesProtected -or $actual.GetOwner([System.Security.Principal.SecurityIdentifier]).Value -ne $sid.Value) { throw 'Private fixture ACL failed' }
}
$source = Join-Path $env:USERPROFILE $suffix
# D: is an existing ephemeral hosted runner volume. Refuse if absent or unsuitable.
$destination = "D:\$suffix"
if ((Get-Volume -DriveLetter C).FileSystem -ne 'NTFS' -or (Get-Volume -DriveLetter D).FileSystem -ne 'NTFS') { throw 'Two existing NTFS volumes required' }
if ([IO.Path]::GetFullPath($source) -ne $source -or [IO.Path]::GetFullPath($destination) -ne $destination) { throw 'Noncanonical fixture path' }
if (-not $source.StartsWith("$env:USERPROFILE\", [StringComparison]::OrdinalIgnoreCase) -or $destination -ne "D:\$suffix") { throw 'Fixture containment failed' }
New-PrivateDirectory $source
New-PrivateDirectory $destination
$env:FOUNDATION_CROSS_VOLUME_SOURCE = $source
$env:FOUNDATION_CROSS_VOLUME_DEST = $destination
"candidate=$env:GITHUB_SHA`nsource_volume=C: NTFS`ndestination_volume=D: NTFS" | Set-Content -LiteralPath "$receipts\native-fixtures.log"
    cargo test -p dendro-core --locked --test native_foundation_acceptance cross_volume_closed_copy_then_local_pointer_publication -- --ignored --exact --nocapture 2>&1 | Tee-Object -FilePath "$receipts\native-fixtures.log" -Append
    $status = $LASTEXITCODE
    if ($status -ne 0) { throw 'Cross-volume native test failed' }
    $log = Get-Content -LiteralPath "$receipts\native-fixtures.log" -Raw
    if (-not $log.Contains('T24-29 actual distinct-device copy, private destination, unchanged closed source and verified local pointer: passed') -or -not $log.Contains('1 passed; 0 failed')) { $status = 1; throw 'Required test execution absent' }
} finally {
    $status | Set-Content -LiteralPath "$receipts\native-fixtures.exit"
    Write-FixtureVerdict
}
