$parameters = [ordered]@{
    Help = $false
    DryRun = $false
    Force = $false
    AllowWrite = $false
    Snapshot = 'after-ac6'
    Test = 'windows_read'
    GuestUser = ''
}
$parseErrors = @()

for ($index = 0; $index -lt $args.Count; $index++) {
    $argument = ([string]$args[$index]).ToLowerInvariant()
    switch -Exact -CaseSensitive ($argument) {
        '-h' { $parameters.Help = $true; break }
        '--help' { $parameters.Help = $true; break }
        '-dryrun' { $parameters.DryRun = $true; break }
        '--dry-run' { $parameters.DryRun = $true; break }
        '-force' { $parameters.Force = $true; break }
        '--force' { $parameters.Force = $true; break }
        '-allowwrite' { $parameters.AllowWrite = $true; break }
        '--allow-write' { $parameters.AllowWrite = $true; break }
        '-snapshot' {
            if ($index + 1 -ge $args.Count) {
                $parseErrors += 'Missing value for -Snapshot.'
            } else {
                $index++
                $parameters.Snapshot = [string]$args[$index]
            }
            break
        }
        '--snapshot' {
            if ($index + 1 -ge $args.Count) {
                $parseErrors += 'Missing value for --snapshot.'
            } else {
                $index++
                $parameters.Snapshot = [string]$args[$index]
            }
            break
        }
        '-test' {
            if ($index + 1 -ge $args.Count) {
                $parseErrors += 'Missing value for -Test.'
            } else {
                $index++
                $parameters.Test = [string]$args[$index]
            }
            break
        }
        '--test' {
            if ($index + 1 -ge $args.Count) {
                $parseErrors += 'Missing value for --test.'
            } else {
                $index++
                $parameters.Test = [string]$args[$index]
            }
            break
        }
        '-guestuser' {
            if ($index + 1 -ge $args.Count) {
                $parseErrors += 'Missing value for -GuestUser.'
            } else {
                $index++
                $parameters.GuestUser = [string]$args[$index]
            }
            break
        }
        '--guest-user' {
            if ($index + 1 -ge $args.Count) {
                $parseErrors += 'Missing value for --guest-user.'
            } else {
                $index++
                $parameters.GuestUser = [string]$args[$index]
            }
            break
        }
        default { $parseErrors += "Unknown argument: $argument" }
    }
}

function Show-Usage {
    @(
        'Usage: powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/run-vm-harness.ps1 [options]'
        ''
        'Options:'
        '  -h, --help                     Show this help.'
        '  -Snapshot, --snapshot <name>   Snapshot to restore. Default: after-ac6.'
        '  -Test, --test <name>            windows_read or windows_write. Default: windows_read.'
        '  -GuestUser, --guest-user <name> Guest account; auto-detected when omitted.'
        '  -DryRun, --dry-run              Print the plan without touching the VM or files.'
        '  -Force, --force                 Restore and start from the requested snapshot if the current snapshot differs.'
        '  -AllowWrite, --allow-write      Allow writes only with host OPTIMI_VM_WRITE_TEST=1.'
        ''
        'The current snapshot name and UUID must match the requested snapshot unless -Force is supplied.'
        'The harness restores and verifies the selected snapshot before and after each run.'
        'windows_write is destructive only with -AllowWrite and host OPTIMI_VM_WRITE_TEST=1; otherwise the guest variable is absent and the test refuses before writes.'
        'windows_write also requires a high-integrity guest process.'
        'Guest test files use C:\vmtest\<run-id> to avoid spaces in %TEMP%.'
        'All host-generated files, Cargo caches, and build outputs are kept under C:/Users/david/VMs/runs.'
    ) | ForEach-Object { [Console]::Out.WriteLine($_) }
}

if ($parameters.Help) {
    Show-Usage
    exit 0
}

if ($parseErrors.Count -gt 0) {
    foreach ($parseError in $parseErrors) {
        [Console]::Error.WriteLine($parseError)
    }
    Show-Usage
    exit 2
}

if (@('windows_read', 'windows_write') -notcontains $parameters.Test) {
    [Console]::Error.WriteLine('Test must be windows_read or windows_write.')
    exit 2
}

if ($parameters.AllowWrite -and $parameters.Test -ne 'windows_write') {
    [Console]::Error.WriteLine('-AllowWrite is only valid with -Test windows_write.')
    exit 2
}

if ([string]::IsNullOrWhiteSpace($parameters.Snapshot)) {
    [Console]::Error.WriteLine('Snapshot name must not be empty.')
    exit 2
}

function Get-GuestWriteGateValue {
    param([string]$Test, [bool]$AllowWrite, [AllowNull()][string]$HostValue)
    if ($Test -eq 'windows_write' -and $AllowWrite -and $HostValue -ceq '1') {
        return $HostValue
    }
    return $null
}

$hostWriteGateValue = [System.Environment]::GetEnvironmentVariable('OPTIMI_VM_WRITE_TEST', 'Process')
$script:WriteGateValue = Get-GuestWriteGateValue -Test $parameters.Test -AllowWrite $parameters.AllowWrite -HostValue $hostWriteGateValue
$script:RunMode = if ($parameters.Test -eq 'windows_read') { 'read_only' } elseif ($null -ne $script:WriteGateValue) { 'destructive' } else { 'non_destructive' }

$script:VmName = 'optimi-test'
$script:VBoxManage = 'C:\Program Files\Oracle\VirtualBox\VBoxManage.exe'
$script:GuestPasswordFile = 'C:\Users\david\VMs\optimi-test\.vm-password'
$script:RunsRoot = 'C:\Users\david\VMs\runs'
$script:Utf8NoBom = [System.Text.UTF8Encoding]::new($false)
$script:GuestUserName = ''
$script:GuestDomain = ''
$script:Record = $null
$script:RecordPath = ''

if ($parameters.DryRun) {
    [Console]::Out.WriteLine('Dry run only. No VM commands or host file writes will be performed.')
    [Console]::Out.WriteLine("VM: $script:VmName")
    [Console]::Out.WriteLine("Snapshot: $($parameters.Snapshot)")
    [Console]::Out.WriteLine("Test: $($parameters.Test)")
    [Console]::Out.WriteLine("Planned mode: $script:RunMode")
    if ($null -eq $script:WriteGateValue) {
        [Console]::Out.WriteLine('Guest OPTIMI_VM_WRITE_TEST: <unset>')
    } else {
        [Console]::Out.WriteLine("Guest OPTIMI_VM_WRITE_TEST: $script:WriteGateValue")
    }
    [Console]::Out.WriteLine('Guest test directory: C:\vmtest\<run-id>')
    [Console]::Out.WriteLine("Force: $($parameters.Force)")
    [Console]::Out.WriteLine('Build: cargo test --locked -p optimi-windows --test ' + $parameters.Test + ' --no-run --message-format=json')
    [Console]::Out.WriteLine('RUSTFLAGS: -C target-feature=+crt-static')
    [Console]::Out.WriteLine("Guest password file: $script:GuestPasswordFile (not read)")
    [Console]::Out.WriteLine("Run directory: $script:RunsRoot")
    exit 0
}

function Assert-RunPath {
    param([string]$Path)
    $root = [System.IO.Path]::GetFullPath($script:RunsRoot).TrimEnd([char[]]@('\', '/'))
    $fullPath = [System.IO.Path]::GetFullPath($Path)
    if (-not $fullPath.StartsWith($root + '\', [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to write outside the runs directory: $fullPath"
    }
}

function Write-RunRecord {
    $json = ConvertTo-Json -InputObject $script:Record -Depth 10
    [System.IO.File]::WriteAllText($script:RecordPath, $json, $script:Utf8NoBom)
}

function Invoke-VBox {
    param([string[]]$Arguments)
    $allArguments = @('--nologo') + $Arguments
    $outputLines = @(& $script:VBoxManage @allArguments 2>&1 | ForEach-Object { $_.ToString() })
    $exitCode = $LASTEXITCODE
    [pscustomobject]@{
        ExitCode = $exitCode
        Output = [string]::Join([System.Environment]::NewLine, $outputLines)
        Arguments = $allArguments
    }
}

function Invoke-VBoxChecked {
    param([string[]]$Arguments)
    $result = Invoke-VBox -Arguments $Arguments
    if ($result.ExitCode -ne 0) {
        throw "VBoxManage exited with code $($result.ExitCode): $($result.Output)"
    }
    $result.Output
}

function ConvertFrom-VBoxValue {
    param([string]$Value)
    $valueText = $Value.Trim()
    if ($valueText.StartsWith('"')) {
        try {
            return ConvertFrom-Json -InputObject $valueText -ErrorAction Stop
        } catch {
            return $valueText.Trim('"')
        }
    }
    $valueText
}

function Get-MachineInfo {
    $text = Invoke-VBoxChecked -Arguments @('showvminfo', $script:VmName, '--machinereadable')
    $properties = @{}
    foreach ($line in ($text -split '\r?\n')) {
        if ($line -match '^(?<key>[^=]+)=(?<value>.*)$') {
            $properties[$matches.key] = ConvertFrom-VBoxValue -Value $matches.value
        }
    }
    $properties
}

function Get-VMState {
    $info = Get-MachineInfo
    [string]$info['VMState']
}

function Get-SnapshotCatalog {
    $text = Invoke-VBoxChecked -Arguments @('snapshot', $script:VmName, 'list', '--machinereadable')
    $snapshotData = @{}
    $currentName = $null
    $currentUuid = $null
    foreach ($line in ($text -split '\r?\n')) {
        if ($line -match '^SnapshotName(?<suffix>(?:-\d+)*)=(?<value>.*)$') {
            $key = $matches.suffix
            if (-not $snapshotData.ContainsKey($key)) {
                $snapshotData[$key] = @{}
            }
            $snapshotData[$key].Name = ConvertFrom-VBoxValue -Value $matches.value
        } elseif ($line -match '^SnapshotUUID(?<suffix>(?:-\d+)*)=(?<value>.*)$') {
            $key = $matches.suffix
            if (-not $snapshotData.ContainsKey($key)) {
                $snapshotData[$key] = @{}
            }
            $snapshotData[$key].UUID = ConvertFrom-VBoxValue -Value $matches.value
        } elseif ($line -match '^CurrentSnapshotName=(?<value>.*)$') {
            $currentName = ConvertFrom-VBoxValue -Value $matches.value
        } elseif ($line -match '^CurrentSnapshotUUID=(?<value>.*)$') {
            $currentUuid = ConvertFrom-VBoxValue -Value $matches.value
        }
    }
    $snapshots = @(
        foreach ($key in $snapshotData.Keys) {
            $entry = $snapshotData[$key]
            if ($entry.ContainsKey('Name') -and $entry.ContainsKey('UUID')) {
                [pscustomobject]@{ Name = $entry.Name; UUID = $entry.UUID }
            }
        }
    )
    [pscustomobject]@{
        Snapshots = $snapshots
        CurrentSnapshotName = $currentName
        CurrentSnapshotUUID = $currentUuid
    }
}

function Wait-ForVMState {
    param([string[]]$ExpectedStates, [int]$TimeoutSeconds = 120)
    $deadline = [System.DateTimeOffset]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $state = Get-VMState
        if ($ExpectedStates -contains $state) {
            return $state
        }
        Start-Sleep -Seconds 2
    } while ([System.DateTimeOffset]::UtcNow -lt $deadline)
    throw "Timed out waiting for VM state $($ExpectedStates -join ', '); last state was $state."
}

function Stop-VMForSnapshotRestore {
    param([ValidateSet('before_run', 'after_run')][string]$Phase = 'after_run')
    $state = Get-VMState
    if ($state -eq 'poweroff') {
        return
    }
    $gracefulError = $null
    if ($state -eq 'paused') {
        try {
            $null = Invoke-VBoxChecked -Arguments @('controlvm', $script:VmName, 'resume')
            $state = 'running'
        } catch {
            $gracefulError = $_.Exception.Message
            if ((Get-VMState) -eq 'poweroff') {
                return
            }
        }
    }
    if ($null -eq $gracefulError -and $state -eq 'saved') {
        try {
            $null = Invoke-VBoxChecked -Arguments @('startvm', $script:VmName, '--type=headless')
            $state = Wait-ForVMState -ExpectedStates @('running', 'poweroff')
        } catch {
            $gracefulError = $_.Exception.Message
            if ((Get-VMState) -eq 'poweroff') {
                return
            }
        }
    }
    if ($null -eq $gracefulError -and $state -eq 'starting') {
        try {
            $state = Wait-ForVMState -ExpectedStates @('running', 'poweroff')
        } catch {
            $gracefulError = $_.Exception.Message
            if ((Get-VMState) -eq 'poweroff') {
                return
            }
        }
    }
    if ($null -eq $gracefulError -and $state -eq 'stopping') {
        try {
            $null = Wait-ForVMState -ExpectedStates @('poweroff')
            return
        } catch {
            $gracefulError = $_.Exception.Message
            if ((Get-VMState) -eq 'poweroff') {
                return
            }
        }
    }
    if ($null -eq $gracefulError -and $state -eq 'poweroff') {
        return
    }
    if ($null -eq $gracefulError -and $state -eq 'running') {
        try {
            $null = Invoke-VBoxChecked -Arguments @('controlvm', $script:VmName, 'acpipowerbutton')
            $null = Wait-ForVMState -ExpectedStates @('poweroff')
            return
        } catch {
            $gracefulError = $_.Exception.Message
            if ((Get-VMState) -eq 'poweroff') {
                return
            }
        }
    }
    if ($null -eq $gracefulError) {
        throw "Refusing snapshot restore while VM state is '$state'."
    }
    $state = Get-VMState
    if ($state -eq 'poweroff') {
        return
    }
    $poweroffArguments = @('controlvm', $script:VmName, 'poweroff')
    $commandName = if ($Phase -eq 'before_run') { 'poweroff_before' } else { 'poweroff_after' }
    $restorationName = if ($Phase -eq 'before_run') { 'poweroff_escalated_before_run' } else { 'poweroff_escalated_after_run' }
    $script:Record.commands[$commandName] = @('VBoxManage', '--nologo') + $poweroffArguments
    try {
        $null = Invoke-VBoxChecked -Arguments $poweroffArguments
    } catch {
        $poweroffError = $_.Exception.Message
        if ((Get-VMState) -eq 'poweroff') {
            return
        }
        throw "Graceful VM shutdown failed: $gracefulError. Poweroff escalation failed: $poweroffError"
    }
    $script:Record.restoration[$restorationName] = $true
    $null = Wait-ForVMState -ExpectedStates @('poweroff')
    [Console]::Error.WriteLine("Graceful VM shutdown did not complete; escalated to poweroff: $gracefulError")
}

function Assert-VMIsPoweredOff {
    $state = Get-VMState
    if ($state -ne 'poweroff') {
        throw "Refusing snapshot restore while VM state is '$state'."
    }
}

function Assert-CurrentSnapshot {
    param($ExpectedSnapshot)
    $catalog = Get-SnapshotCatalog
    if ($catalog.CurrentSnapshotName -cne $ExpectedSnapshot.Name -or $catalog.CurrentSnapshotUUID -ine $ExpectedSnapshot.UUID) {
        $actualName = if ($null -eq $catalog.CurrentSnapshotName) { '<none>' } else { [string]$catalog.CurrentSnapshotName }
        $actualUuid = if ($null -eq $catalog.CurrentSnapshotUUID) { '<none>' } else { [string]$catalog.CurrentSnapshotUUID }
        throw "Refusing to run: current snapshot is '$actualName' ($actualUuid), expected '$($ExpectedSnapshot.Name)' ($($ExpectedSnapshot.UUID))."
    }
    $catalog
}

function Assert-RequestedSnapshot {
    param($Catalog, $ExpectedSnapshot, [switch]$Force)
    if ($Catalog.CurrentSnapshotName -ceq $ExpectedSnapshot.Name -and $Catalog.CurrentSnapshotUUID -ieq $ExpectedSnapshot.UUID) {
        return
    }
    $actualName = if ($null -eq $Catalog.CurrentSnapshotName) { '<none>' } else { [string]$Catalog.CurrentSnapshotName }
    $actualUuid = if ($null -eq $Catalog.CurrentSnapshotUUID) { '<none>' } else { [string]$Catalog.CurrentSnapshotUUID }
    if (-not $Force) {
        throw "Refusing to run: current snapshot is '$actualName' ($actualUuid), requested snapshot is '$($ExpectedSnapshot.Name)' ($($ExpectedSnapshot.UUID)). Use -Force to restore the requested snapshot."
    }
    [Console]::Out.WriteLine("Force enabled; the VM will be restored from requested snapshot '$($ExpectedSnapshot.Name)'.")
}

function Start-VMHeadless {
    $state = Get-VMState
    if ($state -eq 'poweroff' -or $state -eq 'saved') {
        $null = Invoke-VBoxChecked -Arguments @('startvm', $script:VmName, '--type=headless')
    } elseif ($state -ne 'running') {
        throw "Cannot start VM from state '$state'."
    }
    $null = Wait-ForVMState -ExpectedStates @('running')
}

function Get-GuestProperty {
    param([string]$Name)
    $text = Invoke-VBoxChecked -Arguments @('guestproperty', 'get', $script:VmName, $Name)
    $match = [System.Text.RegularExpressions.Regex]::Match($text, '(?m)^Value: ?(?<value>.*)$')
    if (-not $match.Success) {
        throw "Guest property '$Name' is unavailable: $text"
    }
    $match.Groups['value'].Value.Trim()
}

function Resolve-GuestIdentity {
    param([string]$LoggedInUsers)
    $identity = $parameters.GuestUser
    if ([string]::IsNullOrWhiteSpace($identity)) {
        if ([string]::IsNullOrWhiteSpace($LoggedInUsers)) {
            throw 'No logged-in guest account was reported; pass -GuestUser explicitly.'
        }
        $users = @($LoggedInUsers -split '[,;]' | ForEach-Object { $_.Trim() } | Where-Object { $_ })
        if ($users.Count -ne 1) {
            throw 'Multiple or ambiguous guest accounts were reported; pass -GuestUser explicitly.'
        }
        $identity = $users[0]
    }
    if ($identity.Contains('\')) {
        $parts = $identity -split '\\', 2
        if ([string]::IsNullOrWhiteSpace($parts[0]) -or [string]::IsNullOrWhiteSpace($parts[1])) {
            throw 'GuestUser must be a username or DOMAIN\username.'
        }
        $script:GuestDomain = $parts[0]
        $script:GuestUserName = $parts[1]
    } else {
        $script:GuestUserName = $identity
    }
    if ([string]::IsNullOrWhiteSpace($script:GuestUserName)) {
        throw 'GuestUser must not be empty.'
    }
}

function Wait-ForGuestReady {
    param([int]$TimeoutSeconds = 180)
    $deadline = [System.DateTimeOffset]::UtcNow.AddSeconds($TimeoutSeconds)
    $lastGuestAdditionsProbe = 'not checked'
    $lastGuestUsersProbe = 'not checked'
    $lastGuestControlProbe = 'not attempted'
    $guestUserWasSpecified = -not [string]::IsNullOrWhiteSpace($parameters.GuestUser)
    if ($guestUserWasSpecified) {
        Resolve-GuestIdentity -LoggedInUsers ''
    }
    [Console]::Out.WriteLine("Waiting up to $TimeoutSeconds seconds for Guest Additions and the guest desktop.")
    do {
        if ([System.DateTimeOffset]::UtcNow -ge $deadline) {
            break
        }
        $guestAdditionsResult = Invoke-VBox -Arguments @('guestproperty', 'get', $script:VmName, '/VirtualBox/GuestAdd/Version')
        $guestAdditionsMatch = [System.Text.RegularExpressions.Regex]::Match($guestAdditionsResult.Output, '(?m)^Value: ?(?<value>.*)$')
        if ($guestAdditionsResult.ExitCode -eq 0 -and $guestAdditionsMatch.Success) {
            $guestAdditionsVersion = ConvertFrom-VBoxValue -Value $guestAdditionsMatch.Groups['value'].Value
            if (-not [string]::IsNullOrWhiteSpace($guestAdditionsVersion)) {
                $lastGuestAdditionsProbe = $guestAdditionsVersion
                $guestUsersResult = Invoke-VBox -Arguments @('guestproperty', 'get', $script:VmName, '/VirtualBox/GuestInfo/OS/LoggedInUsersList')
                $guestUsersMatch = [System.Text.RegularExpressions.Regex]::Match($guestUsersResult.Output, '(?m)^Value: ?(?<value>.*)$')
                if ($guestUsersResult.ExitCode -eq 0 -and $guestUsersMatch.Success) {
                    $loggedInUsers = ConvertFrom-VBoxValue -Value $guestUsersMatch.Groups['value'].Value
                    $lastGuestUsersProbe = "Value: '$loggedInUsers'"
                    if (-not $guestUserWasSpecified -and [string]::IsNullOrWhiteSpace($script:GuestUserName) -and -not [string]::IsNullOrWhiteSpace($loggedInUsers)) {
                        Resolve-GuestIdentity -LoggedInUsers $loggedInUsers
                    }
                    if ([string]::IsNullOrWhiteSpace($script:GuestUserName)) {
                        $lastGuestControlProbe = 'No logged-in guest account is available for automatic identity detection.'
                    } else {
                        $statArguments = @('guestcontrol', $script:VmName, 'stat', "--username=$script:GuestUserName", "--passwordfile=$script:GuestPasswordFile")
                        if ($script:GuestDomain) {
                            $statArguments += "--domain=$script:GuestDomain"
                        }
                        $statArguments += 'C:\Users\david'
                        $statResult = Invoke-VBox -Arguments $statArguments
                        if ($statResult.ExitCode -eq 0) {
                            if ([System.DateTimeOffset]::UtcNow -lt $deadline) {
                                return $loggedInUsers
                            }
                            $lastGuestControlProbe = 'guestcontrol stat succeeded after the readiness timeout.'
                        } else {
                            $lastGuestControlProbe = "VBoxManage exited with code $($statResult.ExitCode): $($statResult.Output)"
                        }
                    }
                } else {
                    $lastGuestUsersProbe = "VBoxManage exited with code $($guestUsersResult.ExitCode): $($guestUsersResult.Output)"
                    $lastGuestControlProbe = 'Guest identity could not be resolved because the logged-in users property is unavailable.'
                }
            } else {
                $lastGuestAdditionsProbe = $guestAdditionsResult.Output
            }
        } else {
            $lastGuestAdditionsProbe = "VBoxManage exited with code $($guestAdditionsResult.ExitCode): $($guestAdditionsResult.Output)"
        }
        if ([System.DateTimeOffset]::UtcNow -ge $deadline) {
            break
        }
        Start-Sleep -Seconds 2
    } while ($true)
    throw "Timed out after $TimeoutSeconds seconds waiting for Guest Additions and the guest desktop. Last Guest Additions probe: $lastGuestAdditionsProbe. Last logged-in users property probe: $lastGuestUsersProbe. Last guestcontrol stat for C:\Users\david: $lastGuestControlProbe."
}

function Invoke-GuestProcess {
    param([string]$Executable, [string[]]$Arguments = @(), [int]$TimeoutMilliseconds = 120000, [switch]$AllowFailure)
    $guestArguments = @(
        'guestcontrol', $script:VmName, 'run',
        "--exe=$Executable",
        "--username=$script:GuestUserName",
        "--passwordfile=$script:GuestPasswordFile",
        '--wait-stdout', '--wait-stderr', "--timeout=$TimeoutMilliseconds"
    )
    if ($script:GuestDomain) {
        $guestArguments += "--domain=$script:GuestDomain"
    }
    $guestArguments += '--'
    $guestArguments += $Arguments
    $result = Invoke-VBox -Arguments $guestArguments
    if (-not $AllowFailure -and $result.ExitCode -ne 0) {
        throw "Guest command exited with code $($result.ExitCode): $($result.Output)"
    }
    $result
}

function Invoke-GuestFileTransfer {
    param([string[]]$Arguments)
    $result = Invoke-VBox -Arguments $Arguments
    if ($result.ExitCode -ne 0) {
        throw "Guest file transfer exited with code $($result.ExitCode): $($result.Output)"
    }
    $result.Output
}

$startedAt = [System.DateTimeOffset]::UtcNow
$runId = $startedAt.ToString('yyyyMMddTHHmmssfffZ', [System.Globalization.CultureInfo]::InvariantCulture) + '_' + $parameters.Test + '_' + [guid]::NewGuid().ToString('N')
$runDirectory = Join-Path $script:RunsRoot $runId
$recordPath = Join-Path $runDirectory 'run.json'
$cargoHome = Join-Path $script:RunsRoot '.cargo'
$cargoTarget = Join-Path $script:RunsRoot 'cargo-target'
$tempDirectory = Join-Path $runDirectory 'temp'
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))

Assert-RunPath -Path $runDirectory
Assert-RunPath -Path $recordPath
Assert-RunPath -Path $cargoHome
Assert-RunPath -Path $cargoTarget
Assert-RunPath -Path $tempDirectory
[System.IO.Directory]::CreateDirectory($runDirectory) | Out-Null
[System.IO.Directory]::CreateDirectory($cargoHome) | Out-Null
[System.IO.Directory]::CreateDirectory($cargoTarget) | Out-Null
[System.IO.Directory]::CreateDirectory($tempDirectory) | Out-Null

$script:RecordPath = $recordPath
$script:Record = [ordered]@{
    schema_version = 1
    run_id = $runId
    mode = $script:RunMode
    started_at_utc = $startedAt.ToString('o')
    finished_at_utc = $null
    vm = [ordered]@{ name = $script:VmName; uuid = $null; state_before = $null }
    snapshot = [ordered]@{
        requested_name = $parameters.Snapshot
        force_reset = [bool]$parameters.Force
        name = $null
        uuid = $null
        current_before = $null
        verified_before_test = $false
        verified_before_test_at_utc = $null
        current_after = $null
        verified_after_run = $false
        verified_after_run_at_utc = $null
    }
    guest = [ordered]@{
        windows_product = $null
        windows_build = $null
        logged_in_users = $null
        execution_identity = $null
        integrity_groups = $null
    }
    build = [ordered]@{
        command = $null
        rustflags = '-C target-feature=+crt-static'
        exit_code = $null
        artifact = $null
        stdout = $null
        stderr = $null
    }
    test = [ordered]@{
        name = $parameters.Test
        mode = $script:RunMode
        command = $null
        arguments = @('--ignored', '--nocapture', '--test-threads=1')
        environment = [ordered]@{ OPTIMI_VM_WRITE_TEST = $script:WriteGateValue }
        guestcontrol_exit_code = $null
        guestcontrol_output = $null
        exit_code = $null
        stdout = $null
        stderr = $null
    }
    commands = [ordered]@{
        restore_before = $null
        poweroff_before = $null
        copyto_binary = $null
        copyto_wrapper = $null
        test = $null
        copyfrom_stdout = $null
        copyfrom_stderr = $null
        copyfrom_exit_code = $null
        poweroff_after = $null
        restore_after = $null
    }
    restoration = [ordered]@{
        before_run = 'pending'
        after_run = 'pending'
        verified_after_run = $false
        poweroff_escalated_before_run = $false
        poweroff_escalated_after_run = $false
    }
    outcome = 'in_progress'
    errors = @()
}

$targetSnapshot = $null
$snapshotRestoreRequired = $false
$operationError = $null
$restorationError = $null
$recordError = $null

try {
    Write-RunRecord
    if (-not (Test-Path -LiteralPath $script:VBoxManage -PathType Leaf)) {
        throw "VBoxManage was not found at '$script:VBoxManage'."
    }
    if (-not (Test-Path -LiteralPath (Join-Path $repoRoot 'Cargo.toml') -PathType Leaf)) {
        throw "Cargo workspace was not found at '$repoRoot'."
    }
    if (-not (Test-Path -LiteralPath (Join-Path $repoRoot 'Cargo.lock') -PathType Leaf)) {
        throw "Cargo.lock was not found at '$repoRoot'."
    }
    $cargoCommand = Get-Command 'cargo.exe' -ErrorAction SilentlyContinue
    if ($null -eq $cargoCommand) {
        throw 'cargo.exe was not found on PATH.'
    }
    if (-not (Test-Path -LiteralPath $script:GuestPasswordFile -PathType Leaf)) {
        throw "Guest password file was not found at '$script:GuestPasswordFile'."
    }

    $machineInfo = Get-MachineInfo
    $script:Record.vm.uuid = $machineInfo['UUID']
    $script:Record.vm.state_before = $machineInfo['VMState']
    $catalog = Get-SnapshotCatalog
    $matchingSnapshots = @($catalog.Snapshots | Where-Object { $_.Name -ceq $parameters.Snapshot })
    if ($matchingSnapshots.Count -ne 1) {
        throw "Expected exactly one snapshot named '$($parameters.Snapshot)', found $($matchingSnapshots.Count)."
    }
    $targetSnapshot = $matchingSnapshots[0]
    $script:Record.snapshot.name = $targetSnapshot.Name
    $script:Record.snapshot.uuid = $targetSnapshot.UUID
    $script:Record.snapshot.current_before = [ordered]@{
        name = $catalog.CurrentSnapshotName
        uuid = $catalog.CurrentSnapshotUUID
    }
    Assert-RequestedSnapshot -Catalog $catalog -ExpectedSnapshot $targetSnapshot -Force:$parameters.Force
    Write-RunRecord

    $snapshotRestoreRequired = $true
    $null = Stop-VMForSnapshotRestore -Phase before_run
    Assert-VMIsPoweredOff
    $restoreBefore = @('snapshot', $script:VmName, 'restore', $targetSnapshot.UUID)
    $script:Record.commands.restore_before = @('VBoxManage', '--nologo') + $restoreBefore
    $null = Invoke-VBoxChecked -Arguments $restoreBefore
    $verifiedBefore = Assert-CurrentSnapshot -ExpectedSnapshot $targetSnapshot
    $script:Record.snapshot.verified_before_test = $true
    $script:Record.snapshot.verified_before_test_at_utc = [System.DateTimeOffset]::UtcNow.ToString('o')
    $script:Record.snapshot.current_after = [ordered]@{
        name = $verifiedBefore.CurrentSnapshotName
        uuid = $verifiedBefore.CurrentSnapshotUUID
    }
    $script:Record.restoration.before_run = 'verified'
    Write-RunRecord

    Start-VMHeadless
    $loggedInUsers = Wait-ForGuestReady -TimeoutSeconds 180
    $product = Get-GuestProperty -Name '/VirtualBox/GuestInfo/OS/Product'
    $build = Get-GuestProperty -Name '/VirtualBox/GuestInfo/OS/Release'
    $script:Record.guest.windows_product = $product
    $script:Record.guest.windows_build = $build
    $script:Record.guest.logged_in_users = $loggedInUsers

    $identityResult = Invoke-GuestProcess -Executable 'C:\Windows\System32\whoami.exe'
    $identity = $identityResult.Output.Trim()
    if ([string]::IsNullOrWhiteSpace($identity)) {
        throw 'The guest process identity could not be read.'
    }
    $script:Record.guest.execution_identity = $identity

    if ($parameters.Test -eq 'windows_write') {
        $groupsResult = Invoke-GuestProcess -Executable 'C:\Windows\System32\whoami.exe' -Arguments @('/groups')
        $script:Record.guest.integrity_groups = $groupsResult.Output
        if ($groupsResult.Output -notmatch 'S-1-16-12288') {
            throw 'windows_write requires a high-integrity guest process. GuestControl did not provide an elevated token; refusing to run the write test.'
        }
    }
    Write-RunRecord

    $buildArguments = @('test', '--locked', '-p', 'optimi-windows', '--test', $parameters.Test, '--no-run', '--message-format=json')
    $script:Record.build.command = [ordered]@{
        executable = $cargoCommand.Source
        arguments = $buildArguments
        working_directory = $repoRoot
        environment = [ordered]@{ RUSTFLAGS = '-C target-feature=+crt-static'; CARGO_HOME = $cargoHome; CARGO_TARGET_DIR = $cargoTarget }
    }
    $buildStdoutPath = Join-Path $runDirectory 'cargo.stdout'
    $buildStderrPath = Join-Path $runDirectory 'cargo.stderr'
    Assert-RunPath -Path $buildStdoutPath
    Assert-RunPath -Path $buildStderrPath
    $environmentNames = @('CARGO_HOME', 'CARGO_TARGET_DIR', 'RUSTFLAGS', 'TEMP', 'TMP', 'TMPDIR')
    $previousEnvironment = @{}
    foreach ($name in $environmentNames) {
        $previousEnvironment[$name] = [System.Environment]::GetEnvironmentVariable($name, 'Process')
    }
    try {
        [System.Environment]::SetEnvironmentVariable('CARGO_HOME', $cargoHome, 'Process')
        [System.Environment]::SetEnvironmentVariable('CARGO_TARGET_DIR', $cargoTarget, 'Process')
        [System.Environment]::SetEnvironmentVariable('RUSTFLAGS', '-C target-feature=+crt-static', 'Process')
        [System.Environment]::SetEnvironmentVariable('TEMP', $tempDirectory, 'Process')
        [System.Environment]::SetEnvironmentVariable('TMP', $tempDirectory, 'Process')
        [System.Environment]::SetEnvironmentVariable('TMPDIR', $tempDirectory, 'Process')
        $buildProcess = Start-Process -FilePath $cargoCommand.Source -ArgumentList $buildArguments -WorkingDirectory $repoRoot -NoNewWindow -Wait -PassThru -RedirectStandardOutput $buildStdoutPath -RedirectStandardError $buildStderrPath
    } finally {
        foreach ($name in $environmentNames) {
            [System.Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], 'Process')
        }
    }
    $script:Record.build.exit_code = $buildProcess.ExitCode
    $script:Record.build.stdout = [System.IO.File]::ReadAllText($buildStdoutPath, [System.Text.Encoding]::UTF8)
    $script:Record.build.stderr = [System.IO.File]::ReadAllText($buildStderrPath, [System.Text.Encoding]::UTF8)
    Write-RunRecord
    if ($buildProcess.ExitCode -ne 0) {
        throw "Cargo test binary build failed with exit code $($buildProcess.ExitCode)."
    }

    $artifactPaths = @()
    foreach ($line in ($script:Record.build.stdout -split '\r?\n')) {
        if ([string]::IsNullOrWhiteSpace($line)) {
            continue
        }
        try {
            $message = ConvertFrom-Json -InputObject $line -ErrorAction Stop
        } catch {
            continue
        }
        if ($message.reason -eq 'compiler-artifact' -and $message.target.name -eq $parameters.Test -and $message.target.test -eq $true -and $message.executable) {
            $artifactPaths += [string]$message.executable
        }
    }
    $uniqueArtifacts = @($artifactPaths | Select-Object -Unique)
    if ($uniqueArtifacts.Count -ne 1) {
        throw "Expected one compiled test executable for '$($parameters.Test)', found $($uniqueArtifacts.Count)."
    }
    $artifactPath = [System.IO.Path]::GetFullPath($uniqueArtifacts[0])
    if (-not (Test-Path -LiteralPath $artifactPath -PathType Leaf)) {
        throw "Cargo reported a missing test executable: $artifactPath"
    }
    Assert-RunPath -Path $artifactPath
    $script:Record.build.artifact = $artifactPath

    $guestWorkDirectory = [System.IO.Path]::Combine('C:\vmtest', $runId)
    $guestBinaryPath = Join-Path $guestWorkDirectory ($parameters.Test + '.exe')
    $guestWrapperPath = Join-Path $guestWorkDirectory 'run-test.cmd'
    $guestWrapperCommand = '"' + $guestWrapperPath + '"'
    $guestStdoutPath = Join-Path $guestWorkDirectory 'test.stdout'
    $guestStderrPath = Join-Path $guestWorkDirectory 'test.stderr'
    $guestExitCodePath = Join-Path $guestWorkDirectory 'test.exit-code'
    $hostWrapperPath = Join-Path $runDirectory 'run-test.cmd'
    $hostStdoutPath = Join-Path $runDirectory 'test.stdout'
    $hostStderrPath = Join-Path $runDirectory 'test.stderr'
    $hostExitCodePath = Join-Path $runDirectory 'test.exit-code'
    foreach ($path in @($hostWrapperPath, $hostStdoutPath, $hostStderrPath, $hostExitCodePath)) {
        Assert-RunPath -Path $path
    }

    $makeGuestDirectoryArguments = @('guestcontrol', $script:VmName, 'mkdir', '--parents', "--username=$script:GuestUserName", "--passwordfile=$script:GuestPasswordFile")
    if ($script:GuestDomain) {
        $makeGuestDirectoryArguments += "--domain=$script:GuestDomain"
    }
    $makeGuestDirectoryArguments += $guestWorkDirectory
    $null = Invoke-GuestFileTransfer -Arguments $makeGuestDirectoryArguments

    $copyToBinaryArguments = @('guestcontrol', $script:VmName, 'copyto', "--username=$script:GuestUserName", "--passwordfile=$script:GuestPasswordFile")
    if ($script:GuestDomain) {
        $copyToBinaryArguments += "--domain=$script:GuestDomain"
    }
    $copyToBinaryArguments += @($artifactPath, $guestBinaryPath)
    $script:Record.commands.copyto_binary = @('VBoxManage', '--nologo') + $copyToBinaryArguments
    $null = Invoke-GuestFileTransfer -Arguments $copyToBinaryArguments

    $gateValue = $script:WriteGateValue
    $writeGateLine = if ($null -eq $gateValue) { 'set "OPTIMI_VM_WRITE_TEST="' } else { ('set "OPTIMI_VM_WRITE_TEST={0}"' -f $gateValue) }
    $wrapperLines = @(
        '@echo off'
        'setlocal'
        $writeGateLine
        ('"{0}" --ignored --nocapture --test-threads=1 1>"{1}" 2>"{2}"' -f $guestBinaryPath, $guestStdoutPath, $guestStderrPath)
        'set "testExit=%ERRORLEVEL%"'
        ('>"{0}" echo %testExit%' -f $guestExitCodePath)
        'exit /b %testExit%'
    )
    $wrapperText = [string]::Join("`r`n", $wrapperLines) + "`r`n"
    [System.IO.File]::WriteAllText($hostWrapperPath, $wrapperText, [System.Text.Encoding]::ASCII)
    $copyToWrapperArguments = @('guestcontrol', $script:VmName, 'copyto', "--username=$script:GuestUserName", "--passwordfile=$script:GuestPasswordFile")
    if ($script:GuestDomain) {
        $copyToWrapperArguments += "--domain=$script:GuestDomain"
    }
    $copyToWrapperArguments += @($hostWrapperPath, $guestWrapperPath)
    $script:Record.commands.copyto_wrapper = @('VBoxManage', '--nologo') + $copyToWrapperArguments
    $null = Invoke-GuestFileTransfer -Arguments $copyToWrapperArguments

    $testArguments = @('--ignored', '--nocapture', '--test-threads=1')
    $script:Record.test.command = [ordered]@{
        executable = 'C:\Windows\System32\cmd.exe'
        arguments = @('/d', '/v:off', '/c', $guestWrapperCommand)
        test_executable = $guestBinaryPath
        test_arguments = $testArguments
        wrapper = $guestWrapperPath
        environment = [ordered]@{ OPTIMI_VM_WRITE_TEST = $gateValue }
    }
    $testGuestArguments = @(
        'guestcontrol', $script:VmName, 'run',
        '--exe=C:\Windows\System32\cmd.exe',
        "--username=$script:GuestUserName",
        "--passwordfile=$script:GuestPasswordFile",
        '--wait-stdout', '--wait-stderr', '--timeout=900000'
    )
    if ($script:GuestDomain) {
        $testGuestArguments += "--domain=$script:GuestDomain"
    }
    $testGuestArguments += @('--', '/d', '/v:off', '/c', $guestWrapperCommand)
    $script:Record.commands.test = @('VBoxManage') + @('--nologo') + $testGuestArguments
    $testRunResult = Invoke-VBox -Arguments $testGuestArguments
    $script:Record.test.guestcontrol_exit_code = $testRunResult.ExitCode
    $script:Record.test.guestcontrol_output = $testRunResult.Output
    Write-RunRecord

    $authArguments = @("--username=$script:GuestUserName", "--passwordfile=$script:GuestPasswordFile")
    if ($script:GuestDomain) {
        $authArguments += "--domain=$script:GuestDomain"
    }
    $script:Record.commands.copyfrom_stdout = @('VBoxManage', '--nologo', 'guestcontrol', $script:VmName, 'copyfrom') + $authArguments + @($guestStdoutPath, $hostStdoutPath)
    $copyFromStdoutArguments = @('guestcontrol', $script:VmName, 'copyfrom') + $authArguments + @($guestStdoutPath, $hostStdoutPath)
    $null = Invoke-GuestFileTransfer -Arguments $copyFromStdoutArguments
    $script:Record.commands.copyfrom_stderr = @('VBoxManage', '--nologo', 'guestcontrol', $script:VmName, 'copyfrom') + $authArguments + @($guestStderrPath, $hostStderrPath)
    $copyFromStderrArguments = @('guestcontrol', $script:VmName, 'copyfrom') + $authArguments + @($guestStderrPath, $hostStderrPath)
    $null = Invoke-GuestFileTransfer -Arguments $copyFromStderrArguments
    $script:Record.commands.copyfrom_exit_code = @('VBoxManage', '--nologo', 'guestcontrol', $script:VmName, 'copyfrom') + $authArguments + @($guestExitCodePath, $hostExitCodePath)
    $copyFromExitCodeArguments = @('guestcontrol', $script:VmName, 'copyfrom') + $authArguments + @($guestExitCodePath, $hostExitCodePath)
    $null = Invoke-GuestFileTransfer -Arguments $copyFromExitCodeArguments
    $script:Record.test.stdout = [System.IO.File]::ReadAllText($hostStdoutPath, [System.Text.Encoding]::UTF8)
    $script:Record.test.stderr = [System.IO.File]::ReadAllText($hostStderrPath, [System.Text.Encoding]::UTF8)
    $exitCodeText = [System.IO.File]::ReadAllText($hostExitCodePath, [System.Text.Encoding]::UTF8).Trim()
    if ($exitCodeText -notmatch '^-?\d+$') {
        throw "Guest test exit code was not an integer: '$exitCodeText'."
    }
    $script:Record.test.exit_code = [int]$exitCodeText
    Write-RunRecord
} catch {
    $operationError = $_.Exception.Message
    $script:Record.errors += $operationError
} finally {
    if ($null -ne $targetSnapshot -and $snapshotRestoreRequired) {
        try {
            $null = Stop-VMForSnapshotRestore -Phase after_run
            Assert-VMIsPoweredOff
            $restoreAfter = @('snapshot', $script:VmName, 'restore', $targetSnapshot.UUID)
            $script:Record.commands.restore_after = @('VBoxManage', '--nologo') + $restoreAfter
            $null = Invoke-VBoxChecked -Arguments $restoreAfter
            $verifiedAfter = Assert-CurrentSnapshot -ExpectedSnapshot $targetSnapshot
            $script:Record.snapshot.current_after = [ordered]@{
                name = $verifiedAfter.CurrentSnapshotName
                uuid = $verifiedAfter.CurrentSnapshotUUID
            }
            $script:Record.snapshot.verified_after_run = $true
            $script:Record.snapshot.verified_after_run_at_utc = [System.DateTimeOffset]::UtcNow.ToString('o')
            $script:Record.restoration.after_run = 'verified'
            $script:Record.restoration.verified_after_run = $true
        } catch {
            $restorationError = $_.Exception.Message
            $script:Record.restoration.after_run = 'failed'
            $script:Record.errors += "Final snapshot restore failed: $restorationError"
        }
    } elseif ($null -ne $targetSnapshot) {
        $script:Record.restoration.before_run = 'not-required'
        $script:Record.restoration.after_run = 'not-required'
    } else {
        $script:Record.restoration.after_run = 'not-attempted'
    }
    if ($operationError -or $restorationError -or $null -eq $script:Record.test.exit_code) {
        $script:Record.outcome = 'failed'
    } elseif ($script:Record.test.exit_code -eq 0) {
        $script:Record.outcome = 'passed'
    } else {
        $script:Record.outcome = 'test_failed'
    }
    $script:Record.finished_at_utc = [System.DateTimeOffset]::UtcNow.ToString('o')
    try {
        Write-RunRecord
    } catch {
        $recordError = $_.Exception.Message
    }
}

if ($operationError) {
    [Console]::Error.WriteLine("Run failed: $operationError")
}
if ($restorationError) {
    [Console]::Error.WriteLine("Snapshot restoration failed; the VM may remain unrestored: $restorationError")
}
if ($recordError) {
    [Console]::Error.WriteLine("Run record could not be updated: $recordError")
}
[Console]::Out.WriteLine("Record: $recordPath")
if ($operationError -or $restorationError -or $recordError) {
    exit 1
}
if ($script:Record.test.exit_code -ne 0) {
    [Console]::Error.WriteLine("Test '$($parameters.Test)' failed with exit code $($script:Record.test.exit_code).")
    exit [int]$script:Record.test.exit_code
}
[Console]::Out.WriteLine("Test '$($parameters.Test)' passed and snapshot '$($targetSnapshot.Name)' was restored.")
exit 0
