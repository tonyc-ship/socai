$ErrorActionPreference = 'Stop'

$Repo = 'socai-io/socai'
$Asset = 'socai-cli-windows-x86_64.zip'
$Checksum = "$Asset.sha256"
$BaseUrl = if ($env:SOCAI_DOWNLOAD_BASE_URL) { $env:SOCAI_DOWNLOAD_BASE_URL } else { "https://github.com/$Repo/releases/latest/download" }
$InstallDir = if ($env:SOCAI_INSTALL_DIR) { $env:SOCAI_INSTALL_DIR } else { Join-Path $HOME '.socai\bin' }

if ($env:OS -ne 'Windows_NT') {
    throw 'socai Windows CLI installer must be run on Windows. Use install.sh on macOS.'
}

$TempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("socai-install-{0}" -f ([System.Guid]::NewGuid().ToString('N')))
New-Item -ItemType Directory -Force -Path $TempDir | Out-Null

try {
    $ArchivePath = Join-Path $TempDir $Asset
    $ChecksumPath = Join-Path $TempDir $Checksum
    $UnpackDir = Join-Path $TempDir 'unpack'

    Write-Host "downloading socai CLI from $BaseUrl"
    Invoke-WebRequest -UseBasicParsing -Uri "$BaseUrl/$Asset" -OutFile $ArchivePath
    Invoke-WebRequest -UseBasicParsing -Uri "$BaseUrl/$Checksum" -OutFile $ChecksumPath

    $ExpectedHash = ((Get-Content -Raw $ChecksumPath).Trim() -split '\s+')[0].ToLowerInvariant()
    $ActualHash = (Get-FileHash -Algorithm SHA256 $ArchivePath).Hash.ToLowerInvariant()
    if ($ActualHash -ne $ExpectedHash) {
        throw "checksum mismatch for $Asset`: expected $ExpectedHash, got $ActualHash"
    }
    Write-Host "$Asset`: OK"

    New-Item -ItemType Directory -Force -Path $UnpackDir, $InstallDir | Out-Null
    Expand-Archive -Force -Path $ArchivePath -DestinationPath $UnpackDir

    $SourceExe = Join-Path $UnpackDir 'socai.exe'
    if (-not (Test-Path -LiteralPath $SourceExe)) {
        throw 'release archive did not contain socai.exe'
    }

    $DestExe = Join-Path $InstallDir 'socai.exe'
    if (Test-Path -LiteralPath $DestExe) {
        $PreviousSkipUpdateCheck = $env:SOCAI_SKIP_UPDATE_CHECK
        try {
            Write-Host 'stopping existing socai daemon before replacing socai.exe'
            $env:SOCAI_SKIP_UPDATE_CHECK = '1'
            & $DestExe stop | Out-Host
            Start-Sleep -Milliseconds 500
        } catch {
            Write-Warning "could not stop existing socai daemon: $_"
        } finally {
            if ($null -eq $PreviousSkipUpdateCheck) {
                Remove-Item Env:SOCAI_SKIP_UPDATE_CHECK -ErrorAction SilentlyContinue
            } else {
                $env:SOCAI_SKIP_UPDATE_CHECK = $PreviousSkipUpdateCheck
            }
        }
    }
    Copy-Item -Force -LiteralPath $SourceExe -Destination $DestExe

    Write-Host "installed socai to $DestExe"
    & $DestExe --version

    $PathEntries = ($env:PATH -split ';') | Where-Object { $_ -ne '' }
    if ($PathEntries -contains $InstallDir) {
        Write-Host "$InstallDir is already on PATH in this shell"
    } else {
        $env:PATH = "$InstallDir;$env:PATH"
        Write-Host "Added $InstallDir to PATH for this PowerShell session."

        $UserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        $UserEntries = if ($UserPath) { $UserPath -split ';' } else { @() }
        if ($UserEntries -contains $InstallDir) {
            Write-Host "$InstallDir is already on the user PATH"
        } else {
            $NewUserPath = if ($UserPath) { "$InstallDir;$UserPath" } else { $InstallDir }
            [Environment]::SetEnvironmentVariable('Path', $NewUserPath, 'User')
            Write-Host "Updated the user PATH. Open a new terminal to use socai from PATH."
        }
    }
} finally {
    Remove-Item -Recurse -Force -LiteralPath $TempDir -ErrorAction SilentlyContinue
}
