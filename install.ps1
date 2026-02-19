# ZeroAI Installation Script for Windows
# Automatically downloads and installs the latest release for Windows

param(
    [string]$Version,
    [string]$InstallDir = "$env:USERPROFILE\.local\bin",
    [switch]$Force
)

# GitHub repository
$REPO = "hushhenry/zeroai"
$LATEST_RELEASE_URL = "https://api.github.com/repos/$REPO/releases/latest"

# Binary name
$BINARY_NAME = "zeroai-proxy.exe"

# Configuration directory
$CONFIG_DIR = "$env:USERPROFILE\.zeroai"

# Function to write colored output
function Write-Info {
    param([string]$Message)
    Write-Host "[INFO] $Message" -ForegroundColor Cyan
}

function Write-Success {
    param([string]$Message)
    Write-Host "[SUCCESS] $Message" -ForegroundColor Green
}

function Write-Warning {
    param([string]$Message)
    Write-Host "[WARNING] $Message" -ForegroundColor Yellow
}

function Write-Error {
    param([string]$Message)
    Write-Host "[ERROR] $Message" -ForegroundColor Red
}

# Function to detect platform
function Get-Platform {
    $arch = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture
    if ($arch -eq "X64") {
        return "windows-x64"
    } elseif ($arch -eq "Arm64") {
        return "windows-arm64"
    } else {
        Write-Error "Unsupported architecture: $arch"
        exit 1
    }
}

# Function to get latest release version
function Get-LatestVersion {
    Write-Info "Fetching latest release information..."
    
    try {
        $response = Invoke-RestMethod -Uri $LATEST_RELEASE_URL -Method Get
        return $response.tag_name
    } catch {
        Write-Error "Failed to fetch release info: $($_.Exception.Message)"
        exit 1
    }
}

# Function to download binary
function Download-Binary {
    param(
        [string]$Platform,
        [string]$Version
    )
    
    Write-Info "Downloading ZeroAI $Version for $Platform..."
    
    # Determine binary name based on platform
    $binaryFile = switch ($Platform) {
        "windows-x64" { "zeroai-proxy-windows-x64.exe" }
        "windows-arm64" { "zeroai-proxy-windows-arm64.exe" }
        default {
            Write-Error "Unsupported platform: $Platform"
            exit 1
        }
    }
    
    # Download URL
    $downloadUrl = "https://github.com/$REPO/releases/download/$Version/$binaryFile"
    
    # Create temporary directory
    $tempDir = Join-Path $env:TEMP "zeroai_install"
    New-Item -ItemType Directory -Force -Path $tempDir | Out-Null
    $tempFile = Join-Path $tempDir $binaryFile
    
    # Download the binary
    try {
        Invoke-WebRequest -Uri $downloadUrl -OutFile $tempFile
    } catch {
        Write-Error "Failed to download binary from $downloadUrl"
        Remove-Item -Path $tempDir -Recurse -Force
        exit 1
    }
    
    return $tempFile
}

# Function to install binary
function Install-Binary {
    param(
        [string]$TempFile,
        [string]$Platform
    )
    
    Write-Info "Installing binary to $InstallDir..."
    
    # Create installation directory if it doesn't exist
    if (-not (Test-Path $InstallDir)) {
        New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    }
    
    # Determine binary name based on platform
    $binaryName = $BINARY_NAME
    
    # Move binary to installation directory
    $destinationPath = Join-Path $InstallDir $binaryName
    Move-Item -Path $TempFile -Destination $destinationPath -Force
    
    # Add to PATH if not already there
    $currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($currentPath -notlike "*$InstallDir*") {
        Write-Warning "Installation directory $InstallDir is not in your PATH."
        Write-Warning "Add the following line to your PATH environment variable:"
        Write-Host "  $InstallDir" -ForegroundColor Yellow
    }
}

# Function to create config directory
function Create-ConfigDir {
    Write-Info "Creating configuration directory..."
    
    if (-not (Test-Path $CONFIG_DIR)) {
        New-Item -ItemType Directory -Force -Path $CONFIG_DIR | Out-Null
    }
    
    # Create initial config file if it doesn't exist
    $configFile = Join-Path $CONFIG_DIR "config.json"
    if (-not (Test-Path $configFile)) {
        $configContent = @'
{
  "providers": {}
}
'@
        Set-Content -Path $configFile -Value $configContent
        Write-Success "Created initial configuration file at $configFile"
    }
}

# Function to show usage
function Show-Usage {
    Write-Host "Usage: .\install.ps1 [OPTIONS]" -ForegroundColor White
    Write-Host ""
    Write-Host "Options:" -ForegroundColor White
    Write-Host "  -Version <VERSION>    Version to install (default: latest)" -ForegroundColor Gray
    Write-Host "  -InstallDir <DIR>     Installation directory (default: $env:USERPROFILE\.local\bin)" -ForegroundColor Gray
    Write-Host "  -Force                Force installation even if already installed" -ForegroundColor Gray
    Write-Host "  -Help                 Show this help message" -ForegroundColor Gray
    Write-Host ""
    Write-Host "Examples:" -ForegroundColor White
    Write-Host "  .\install.ps1                    # Install latest version" -ForegroundColor Gray
    Write-Host "  .\install.ps1 -Version v0.1.0    # Install specific version" -ForegroundColor Gray
    Write-Host "  .\install.ps1 -InstallDir C:\tools  # Install to custom directory" -ForegroundColor Gray
}

# Main installation function
function Main {
    # Show help if requested
    if ($Help) {
        Show-Usage
        exit 0
    }
    
    # Detect platform
    $platform = Get-Platform
    Write-Info "Detected platform: $platform"
    
    # Get latest version if not specified
    if (-not $Version) {
        $Version = Get-LatestVersion
        Write-Info "Latest version: $Version"
    } else {
        Write-Info "Target version: $Version"
    }
    
    # Check if already installed
    $binaryPath = Join-Path $InstallDir $BINARY_NAME
    if ((Test-Path $binaryPath) -and (-not $Force)) {
        Write-Warning "ZeroAI is already installed at $binaryPath"
        Write-Warning "Use -Force to reinstall or update"
        exit 0
    }
    
    # Download binary
    $tempFile = Download-Binary -Platform $platform -Version $Version
    
    # Install binary
    Install-Binary -TempFile $tempFile -Platform $platform
    
    # Create config directory
    Create-ConfigDir
    
    # Verify installation
    if (Test-Path $binaryPath) {
        Write-Success "ZeroAI installed successfully!"
        Write-Info "Run 'zeroai-proxy.exe --help' to see available commands"
    } else {
        Write-Warning "Binary installation may have failed. Please check manually."
    }
    
    # Show next steps
    Write-Host ""
    Write-Info "Next steps:"
    Write-Host "  1. Add $InstallDir to your PATH if not already done" -ForegroundColor Gray
    Write-Host "  2. Run 'zeroai-proxy.exe config' to configure providers" -ForegroundColor Gray
    Write-Host "  3. Run 'zeroai-proxy.exe serve' to start the proxy server" -ForegroundColor Gray
}

# Run main function
Main